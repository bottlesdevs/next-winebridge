//! A tonic transport that never registers a socket with tokio's I/O reactor.
//!
//! Wine older than 7.13 rejects `IOCTL_AFD_POLL` on the standalone `\Device\Afd`
//! handle mio's reactor depends on (`STATUS_BAD_DEVICE_TYPE`, surfacing as a
//! bare `os error 66`), so a normal [`tonic::transport::server::TcpIncoming`]
//! cannot even bind under such a Wine. Blocking `std::net` calls are unaffected
//! by that restriction and work fully — accept, read, and write in both
//! directions — so this transport accepts and moves bytes on dedicated OS
//! threads and bridges the results into `AsyncRead`/`AsyncWrite` through
//! [`tokio::task::spawn_blocking`], never touching mio at all.
//!
//! [`Incoming`] chooses between this and the normal tokio transport at bind
//! time, so a host with working Wine keeps the native, more efficient reactor.

use std::{
    future::Future,
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener as StdTcpListener, TcpStream as StdTcpStream},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures_core::Stream;
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::mpsc,
    task::JoinHandle,
};
use tonic::transport::server::{Connected, TcpConnectInfo, TcpIncoming};

/// Caps the size of one blocking read, bounding the buffer `poll_read`
/// allocates per call regardless of how much the caller's [`ReadBuf`] can hold.
const READ_CHUNK: usize = 64 * 1024;

/// Either transport, selected once at bind time.
pub enum Incoming {
    Tokio(TcpIncoming),
    Blocking(BlockingIncoming),
}

impl Incoming {
    /// Binds `addr` with the tokio transport, or the blocking one when
    /// `blocking` is set — pass `true` for a runner whose Wine cannot poll
    /// sockets.
    pub fn bind(addr: SocketAddr, blocking: bool) -> io::Result<Self> {
        if blocking {
            Ok(Self::Blocking(BlockingIncoming::bind(addr)?))
        } else {
            Ok(Self::Tokio(TcpIncoming::bind(addr)?))
        }
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        match self {
            Self::Tokio(incoming) => incoming.local_addr(),
            Self::Blocking(incoming) => Ok(incoming.local_addr()),
        }
    }
}

impl Stream for Incoming {
    type Item = io::Result<EitherStream>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.get_mut() {
            Self::Tokio(incoming) => Pin::new(incoming)
                .poll_next(cx)
                .map_ok(EitherStream::Tokio),
            Self::Blocking(incoming) => Pin::new(incoming)
                .poll_next(cx)
                .map_ok(EitherStream::Blocking),
        }
    }
}

/// A connection accepted by either transport.
pub enum EitherStream {
    Tokio(tokio::net::TcpStream),
    Blocking(BlockingStream),
}

impl Connected for EitherStream {
    type ConnectInfo = TcpConnectInfo;

    fn connect_info(&self) -> Self::ConnectInfo {
        match self {
            Self::Tokio(stream) => stream.connect_info(),
            Self::Blocking(stream) => stream.connect_info(),
        }
    }
}

impl AsyncRead for EitherStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tokio(stream) => Pin::new(stream).poll_read(cx, buf),
            Self::Blocking(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for EitherStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Tokio(stream) => Pin::new(stream).poll_write(cx, data),
            Self::Blocking(stream) => Pin::new(stream).poll_write(cx, data),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tokio(stream) => Pin::new(stream).poll_flush(cx),
            Self::Blocking(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tokio(stream) => Pin::new(stream).poll_shutdown(cx),
            Self::Blocking(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

/// Accepts connections on a dedicated OS thread using blocking `accept()`,
/// forwarding each to the async side through a channel.
///
/// The listening socket is polled only by that thread's blocking `accept()`
/// call, so — unlike [`TcpIncoming`] — it never registers with mio's reactor.
/// The accept thread outlives this value; it is not joined on drop, since
/// WineBridge is a short-lived per-prefix process and the thread exits with
/// it regardless.
pub struct BlockingIncoming {
    local_addr: SocketAddr,
    accepted: mpsc::UnboundedReceiver<io::Result<BlockingStream>>,
}

impl BlockingIncoming {
    pub fn bind(addr: SocketAddr) -> io::Result<Self> {
        let listener = StdTcpListener::bind(addr)?;
        let local_addr = listener.local_addr()?;
        let (sender, accepted) = mpsc::unbounded_channel();
        std::thread::spawn(move || {
            loop {
                let accepted = listener
                    .accept()
                    .and_then(|(stream, peer)| BlockingStream::new(stream, local_addr, peer));
                // A send failure means the async side is gone; an accept
                // failure is treated as fatal for this listener, matching
                // TcpListenerStream's behavior of surfacing the error once.
                let stop = accepted.is_err();
                if sender.send(accepted).is_err() || stop {
                    break;
                }
            }
        });
        Ok(Self {
            local_addr,
            accepted,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
}

impl Stream for BlockingIncoming {
    type Item = io::Result<BlockingStream>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.accepted.poll_recv(cx)
    }
}

/// One accepted connection, read and written through blocking calls on
/// [`tokio::task::spawn_blocking`] rather than through the I/O reactor.
pub struct BlockingStream {
    connect_info: TcpConnectInfo,
    read: ReadHalf,
    write: WriteHalf,
}

impl BlockingStream {
    fn new(stream: StdTcpStream, local_addr: SocketAddr, peer_addr: SocketAddr) -> io::Result<Self> {
        let read_side = Arc::new(stream.try_clone()?);
        let write_side = Arc::new(stream);
        Ok(Self {
            connect_info: TcpConnectInfo {
                local_addr: Some(local_addr),
                remote_addr: Some(peer_addr),
            },
            read: ReadHalf {
                stream: read_side,
                state: ReadState::Idle,
            },
            write: WriteHalf {
                stream: write_side,
                state: WriteState::Idle,
            },
        })
    }
}

impl Connected for BlockingStream {
    type ConnectInfo = TcpConnectInfo;

    fn connect_info(&self) -> Self::ConnectInfo {
        self.connect_info.clone()
    }
}

impl AsyncRead for BlockingStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().read).poll_read(cx, buf)
    }
}

impl AsyncWrite for BlockingStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().write).poll_write(cx, data)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().write).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().write).poll_shutdown(cx)
    }
}

/// One in-flight blocking read, at most [`READ_CHUNK`] bytes.
enum ReadState {
    Idle,
    Reading(JoinHandle<io::Result<Vec<u8>>>),
}

struct ReadHalf {
    stream: Arc<StdTcpStream>,
    state: ReadState,
}

impl AsyncRead for ReadHalf {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            match &mut self.state {
                ReadState::Idle => {
                    // A zero-length read on a TcpStream returns Ok(0), which
                    // AsyncRead reserves to mean EOF, so at least one byte is
                    // always requested even if the caller's buffer is full.
                    let want = buf.remaining().clamp(1, READ_CHUNK);
                    let stream = self.stream.clone();
                    self.state = ReadState::Reading(tokio::task::spawn_blocking(move || {
                        let mut chunk = vec![0u8; want];
                        let read = (&*stream).read(&mut chunk)?;
                        chunk.truncate(read);
                        Ok(chunk)
                    }));
                }
                ReadState::Reading(handle) => {
                    let joined = match Pin::new(handle).poll(cx) {
                        Poll::Ready(joined) => joined,
                        Poll::Pending => return Poll::Pending,
                    };
                    self.state = ReadState::Idle;
                    return Poll::Ready(match joined {
                        Ok(Ok(chunk)) => {
                            buf.put_slice(&chunk);
                            Ok(())
                        }
                        Ok(Err(error)) => Err(error),
                        Err(join_error) => Err(io::Error::other(join_error)),
                    });
                }
            }
        }
    }
}

/// One in-flight blocking write of the caller's most recent `poll_write` data.
enum WriteState {
    Idle,
    Writing(JoinHandle<io::Result<usize>>),
}

struct WriteHalf {
    stream: Arc<StdTcpStream>,
    state: WriteState,
}

impl AsyncWrite for WriteHalf {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            match &mut self.state {
                WriteState::Idle => {
                    let stream = self.stream.clone();
                    let chunk = data.to_vec();
                    self.state = WriteState::Writing(tokio::task::spawn_blocking(move || {
                        (&*stream).write(&chunk)
                    }));
                }
                WriteState::Writing(handle) => {
                    let joined = match Pin::new(handle).poll(cx) {
                        Poll::Ready(joined) => joined,
                        Poll::Pending => return Poll::Pending,
                    };
                    self.state = WriteState::Idle;
                    return Poll::Ready(match joined {
                        Ok(result) => result,
                        Err(join_error) => Err(io::Error::other(join_error)),
                    });
                }
            }
        }
    }

    /// A no-op: `std::net::TcpStream` has no userspace write buffering to flush.
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    /// Half-closes the write side. `TcpStream::shutdown` is a quick socket
    /// state change rather than a network round trip, so it runs inline
    /// instead of through `spawn_blocking`.
    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(self.stream.shutdown(std::net::Shutdown::Write))
    }
}
