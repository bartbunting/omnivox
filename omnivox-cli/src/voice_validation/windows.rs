use crate::remote_windows::Job;
use std::io;
use std::process::{Child, Command};

pub fn initialize() -> io::Result<()> {
    Ok(())
}
pub fn configure(_: &mut Command, _: usize) {}
pub fn configure_speech(_: &mut Command) {}
pub fn check_worker_group() -> io::Result<()> {
    Ok(())
}
pub fn parent_closed() -> ! {
    std::process::exit(1)
}

pub struct Tree(Job);
impl Tree {
    pub fn for_speech() -> io::Result<Self> {
        Job::new().map(Self).map_err(io::Error::other)
    }
    pub fn new(memory: usize) -> io::Result<Self> {
        Job::for_validation(memory)
            .map(Self)
            .map_err(io::Error::other)
    }
    pub fn assign(&mut self, child: &Child) -> io::Result<()> {
        self.0.assign(child).map_err(io::Error::other)
    }
    pub fn terminate(&mut self) -> io::Result<()> {
        self.0.terminate_checked()
    }
    pub fn empty(&mut self) -> io::Result<bool> {
        self.0.is_empty()
    }
}
