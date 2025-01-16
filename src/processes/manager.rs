use super::process::{Process, ProcessSnapshot};
use windows::core::Error;

struct ProcessManager;

impl ProcessManager {
    fn running_processes(&self) -> Result<Vec<Process>, Error> {
        let snapshot = ProcessSnapshot::new()?;

        Ok(snapshot.map(|process| process).collect())
    }
}
