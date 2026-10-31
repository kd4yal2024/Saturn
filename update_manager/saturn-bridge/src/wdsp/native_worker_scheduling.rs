//! Native pthreads inherit their creator's scheduler and affinity. Construct
//! (and recreate) managed WDSP channels outside the TX owner's FIFO domain.
//! Only the creator is temporarily changed; existing native workers are never
//! discovered/retuned by name, and active DSP is never moved between owners.
use saturn_bridge::puresignal::Fault;

#[derive(Clone)]
pub(crate) struct WorkerPolicy {
    cpus: Vec<usize>,
}

impl WorkerPolicy {
    /// Capture BEFORE the owner is pinned. Reserve its last CPU when realtime
    /// scheduling is selected, provided the process has another allowed CPU.
    pub(crate) fn capture(reserve_owner_cpu: bool) -> Result<Self, Fault> {
        let mut cpus = crate::xdma_duc::allowed_cpu_ids().map_err(|_| Fault::NativeFailure)?;
        if reserve_owner_cpu && cpus.len() > 1 {
            cpus.pop();
        }
        Ok(Self { cpus })
    }

    /// RF must be stopped before entering this construction-only scope.
    /// Explicit restoration failures propagate: they cannot declare startup or
    /// cleanup successful. Drop also restores on an early return or unwind.
    pub(crate) fn construct<T>(&self, operation: impl FnOnce() -> T) -> Result<T, Fault> {
        let mut guard = Restore::capture()?;
        set_scheduler(libc::SCHED_OTHER, 0)?;
        set_affinity(&self.cpus)?;
        if crate::xdma_duc::current_scheduler().map_err(|_| Fault::NativeFailure)? != ("other", 0)
            || crate::xdma_duc::allowed_cpu_ids().map_err(|_| Fault::NativeFailure)? != self.cpus
        {
            return Err(Fault::NativeFailure);
        }
        let result = operation();
        guard.restore()?;
        Ok(result)
    }
}

struct Restore {
    cpus: Vec<usize>,
    policy: i32,
    priority: i32,
    restored: bool,
}
impl Restore {
    fn capture() -> Result<Self, Fault> {
        let cpus = crate::xdma_duc::allowed_cpu_ids().map_err(|_| Fault::NativeFailure)?;
        // SAFETY: pid zero means this thread; parameter storage is initialized.
        let (policy, priority) = unsafe {
            let policy = libc::sched_getscheduler(0);
            let mut parameter: libc::sched_param = std::mem::zeroed();
            if policy < 0 || libc::sched_getparam(0, &mut parameter) != 0 {
                return Err(Fault::NativeFailure);
            }
            (policy, parameter.sched_priority)
        };
        if !matches!(
            policy,
            libc::SCHED_OTHER
                | libc::SCHED_BATCH
                | libc::SCHED_IDLE
                | libc::SCHED_FIFO
                | libc::SCHED_RR
        ) {
            return Err(Fault::NativeFailure);
        }
        Ok(Self {
            cpus,
            policy,
            priority,
            restored: false,
        })
    }
    fn restore(&mut self) -> Result<(), Fault> {
        // Restore affinity BEFORE re-entering FIFO, so restoration itself does
        // not temporarily run with realtime priority on acquisition CPUs.
        set_affinity(&self.cpus)?;
        set_scheduler(self.policy, self.priority)?;
        let actual_cpus = crate::xdma_duc::allowed_cpu_ids().map_err(|_| Fault::NativeFailure)?;
        let actual_policy = unsafe { libc::sched_getscheduler(0) };
        let mut parameter: libc::sched_param = unsafe { std::mem::zeroed() };
        if unsafe { libc::sched_getparam(0, &mut parameter) } != 0
            || actual_policy != self.policy
            || parameter.sched_priority != self.priority
            || actual_cpus != self.cpus
        {
            return Err(Fault::NativeFailure);
        }
        self.restored = true;
        Ok(())
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        if !self.restored && self.restore().is_err() {
            eprintln!("managed WDSP creator scheduling restoration failed; native startup/cleanup unavailable");
        }
    }
}

fn set_scheduler(policy: i32, priority: i32) -> Result<(), Fault> {
    let parameter = libc::sched_param {
        sched_priority: priority,
    };
    if unsafe { libc::sched_setscheduler(0, policy, &parameter) } != 0 {
        return Err(Fault::NativeFailure);
    }
    Ok(())
}
fn set_affinity(cpus: &[usize]) -> Result<(), Fault> {
    if cpus.is_empty() || cpus.iter().any(|c| *c >= libc::CPU_SETSIZE as usize) {
        return Err(Fault::NativeFailure);
    }
    let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
    for cpu in cpus {
        unsafe { libc::CPU_SET(*cpu, &mut set) };
    }
    if unsafe { libc::sched_setaffinity(0, std::mem::size_of_val(&set), &set) } != 0 {
        return Err(Fault::NativeFailure);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inherited_workers_are_other_and_creator_is_restored_on_success_error_and_unwind() {
        std::thread::spawn(|| {
            let policy = WorkerPolicy::capture(true).unwrap();
            let before = crate::xdma_duc::allowed_cpu_ids().unwrap();
            // BATCH needs no realtime privilege but exposes inheritance and
            // exact restoration. This does not claim a FIFO timing test.
            set_scheduler(libc::SCHED_BATCH, 0).unwrap();
            set_affinity(&[*before.last().unwrap()]).unwrap();
            let parent = crate::xdma_duc::allowed_cpu_ids().unwrap();
            for fail in [false, true] {
                let result = policy
                    .construct(|| {
                        let expected = policy.cpus.clone();
                        std::thread::spawn(move || {
                            assert_eq!(crate::xdma_duc::current_scheduler().unwrap(), ("other", 0));
                            assert_eq!(crate::xdma_duc::allowed_cpu_ids().unwrap(), expected);
                        })
                        .join()
                        .unwrap();
                        if fail {
                            Err(Fault::NativeFailure)
                        } else {
                            Ok(())
                        }
                    })
                    .unwrap();
                assert_eq!(result.is_err(), fail);
                assert_eq!(crate::xdma_duc::current_scheduler().unwrap(), ("batch", 0));
                assert_eq!(crate::xdma_duc::allowed_cpu_ids().unwrap(), parent);
            }
            // A failed affinity transition must restore the creator too; the
            // native constructor must never run after that failure.
            assert!(matches!(
                WorkerPolicy { cpus: vec![] }.construct(|| panic!("must not construct")),
                Err(Fault::NativeFailure)
            ));
            assert_eq!(crate::xdma_duc::current_scheduler().unwrap(), ("batch", 0));
            assert_eq!(crate::xdma_duc::allowed_cpu_ids().unwrap(), parent);
            assert!(
                std::panic::catch_unwind(|| policy.construct(|| panic!("construction unwind")))
                    .is_err()
            );
            assert_eq!(crate::xdma_duc::current_scheduler().unwrap(), ("batch", 0));
            assert_eq!(crate::xdma_duc::allowed_cpu_ids().unwrap(), parent);
        })
        .join()
        .unwrap();
    }
}
