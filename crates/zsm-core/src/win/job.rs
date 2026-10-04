use std::io;
use std::os::windows::io::OwnedHandle;
use windows::core::PCWSTR;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicProcessIdList,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

use super::{io_err, owned, raw, Process};

/// Job object that kills every process in it when the last handle closes.
pub struct Job(OwnedHandle);

impl Job {
    pub fn new() -> io::Result<Job> {
        let handle = unsafe { CreateJobObjectW(None, PCWSTR::null()) }.map_err(io_err)?;
        let job = Job(owned(handle));
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                raw(&job.0),
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of_val(&info) as u32,
            )
        }
        .map_err(io_err)?;
        Ok(job)
    }

    pub fn assign(&self, process: &Process) -> io::Result<()> {
        unsafe { AssignProcessToJobObject(raw(&self.0), process.raw()) }.map_err(io_err)
    }

    pub fn terminate(&self, exit_code: u32) -> io::Result<()> {
        unsafe { TerminateJobObject(raw(&self.0), exit_code) }.map_err(io_err)
    }

    pub fn process_ids(&self) -> io::Result<Vec<u32>> {
        let mut capacity = 64usize;
        loop {
            // JOBOBJECT_BASIC_PROCESS_ID_LIST: two u32 counters followed by a ULONG_PTR array.
            let mut buf = vec![0usize; 1 + capacity];
            let res = unsafe {
                QueryInformationJobObject(
                    Some(raw(&self.0)),
                    JobObjectBasicProcessIdList,
                    buf.as_mut_ptr() as *mut _,
                    (buf.len() * std::mem::size_of::<usize>()) as u32,
                    None,
                )
            };
            let assigned = buf[0] & 0xFFFF_FFFF;
            let listed = buf[0] >> 32;
            match res {
                Ok(()) => return Ok(buf[1..1 + listed].iter().map(|&p| p as u32).collect()),
                Err(_) if assigned > capacity => capacity = assigned + 16,
                Err(e) => return Err(io_err(e)),
            }
        }
    }
}
