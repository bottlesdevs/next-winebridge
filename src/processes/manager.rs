use super::process::{Process, ProcessIdentifier, ProcessSnapshot};
use windows::core::Error;

struct ProcessManager;

impl ProcessManager {
    fn running_processes(&self) -> Result<Vec<Process>, Error> {
        let snapshot = ProcessSnapshot::new()?;

        Ok(snapshot.map(|process| process).collect())
    }

    pub fn process(&self, identifier: ProcessIdentifier) -> Option<Process> {
        let processes = self
            .running_processes()
            .expect("Failed to get running processes");

        match identifier {
            ProcessIdentifier::Name(name) => processes
                .iter()
                .find(|p| p.name().to_lowercase() == name.to_lowercase())
                .cloned(),
            ProcessIdentifier::PID(pid) => processes.iter().find(|p| p.pid() == pid).cloned(),
        }
    }
}
