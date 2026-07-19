use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{RecvTimeoutError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const HELPER_CHANNEL_CAPACITY: usize = 1;
const HELPER_THREAD_NAME: &str = "frame-sck-native-call";

// A timed-out ScreenCaptureKit completion wait may never return. Keeping this
// process-wide lease in the helper hard-caps stuck native waits and stranded
// stream owners at one. A helper can release the lease only after its native
// call returns; its final result send/return may briefly overlap the next call.
static NATIVE_CALL_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

struct NativeCallLease;

impl NativeCallLease {
    fn acquire() -> Option<Self> {
        NATIVE_CALL_IN_FLIGHT
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self)
    }
}

impl Drop for NativeCallLease {
    fn drop(&mut self) {
        NATIVE_CALL_IN_FLIGHT.store(false, Ordering::Release);
    }
}

pub(super) struct PendingNativeCall {
    // Dropping a JoinHandle detaches. Never join this worker: it may be stuck
    // forever inside the dependency's unbounded Condvar wait.
    _worker: JoinHandle<()>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NativeCallLaunchError {
    CapacityUnavailable,
    WorkerUnavailable,
}

pub(super) enum BoundedNativeCall<T, R> {
    Completed {
        owner: T,
        result: R,
    },
    NotStarted {
        owner: T,
        error: NativeCallLaunchError,
    },
    Unconfirmed(PendingNativeCall),
}

pub(super) fn run_bounded_native_call<T, R, F>(
    owner: T,
    timeout: Duration,
    operation: F,
) -> BoundedNativeCall<T, R>
where
    T: Send + 'static,
    R: Send + 'static,
    F: FnOnce(T) -> (T, R) + Send + 'static,
{
    let Some(lease) = NativeCallLease::acquire() else {
        return BoundedNativeCall::NotStarted {
            owner,
            error: NativeCallLaunchError::CapacityUnavailable,
        };
    };
    let (start_sender, start_receiver) =
        sync_channel::<(T, F, NativeCallLease)>(HELPER_CHANNEL_CAPACITY);
    let (completion_sender, completion_receiver) =
        sync_channel::<(T, R, NativeCallLease)>(HELPER_CHANNEL_CAPACITY);
    let worker = thread::Builder::new()
        .name(HELPER_THREAD_NAME.into())
        .spawn(move || {
            let Ok((owner, operation, lease)) = start_receiver.recv() else {
                return;
            };
            let (owner, result) = operation(owner);
            // A timeout drops the receiver. Keep the process-wide lease while
            // explicitly destroying every rejected result owner so a blocking
            // late SCStream drop cannot overlap another native call.
            if let Err(error) = completion_sender.send((owner, result, lease)) {
                let (owner, result, lease) = error.0;
                drop(result);
                drop(owner);
                drop(lease);
            }
        });
    let worker = match worker {
        Ok(worker) => worker,
        Err(_) => {
            return BoundedNativeCall::NotStarted {
                owner,
                error: NativeCallLaunchError::WorkerUnavailable,
            };
        }
    };
    if let Err(error) = start_sender.send((owner, operation, lease)) {
        let (owner, _, _) = error.0;
        // The worker can only observe a disconnected start channel and exit.
        // Detach instead of joining so this path follows the same ownership
        // rule as every other helper outcome.
        drop(worker);
        return BoundedNativeCall::NotStarted {
            owner,
            error: NativeCallLaunchError::WorkerUnavailable,
        };
    }
    match completion_receiver.recv_timeout(timeout) {
        Ok((owner, result, lease)) => {
            // The native wait has returned. The helper may still be completing
            // its final send/return, so its handle is deliberately detached.
            drop(worker);
            let completed = BoundedNativeCall::Completed { owner, result };
            // The receiving thread recovers both values before reopening
            // process-wide native-call capacity.
            drop(lease);
            completed
        }
        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
            drop(completion_receiver);
            BoundedNativeCall::Unconfirmed(PendingNativeCall { _worker: worker })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc::{Receiver, SyncSender, sync_channel},
        },
        thread,
        time::{Duration, Instant},
    };

    use super::*;

    struct BlockingDropProbe {
        dropped: Arc<AtomicBool>,
        drop_started: SyncSender<()>,
        allow_drop: Receiver<()>,
    }

    impl Drop for BlockingDropProbe {
        fn drop(&mut self) {
            self.drop_started
                .send(())
                .expect("report blocking owner destruction");
            self.allow_drop
                .recv()
                .expect("release blocking owner destruction");
            self.dropped.store(true, Ordering::Release);
        }
    }

    #[test]
    fn timeout_holds_capacity_until_late_owner_destruction_finishes() {
        let completed = run_bounded_native_call(3_u8, Duration::from_secs(5), |owner| {
            (owner, owner.saturating_add(1))
        });
        let BoundedNativeCall::Completed { owner, result } = completed else {
            panic!("immediate native model call must complete");
        };
        assert_eq!((owner, result), (3, 4));

        let (allow_call, wait_for_call) = sync_channel(HELPER_CHANNEL_CAPACITY);
        let (drop_started, wait_for_drop_start) = sync_channel(HELPER_CHANNEL_CAPACITY);
        let (allow_drop, wait_for_drop) = sync_channel(HELPER_CHANNEL_CAPACITY);
        let owner_dropped = Arc::new(AtomicBool::new(false));
        let timed_out = run_bounded_native_call(
            BlockingDropProbe {
                dropped: Arc::clone(&owner_dropped),
                drop_started,
                allow_drop: wait_for_drop,
            },
            Duration::from_millis(1),
            move |owner| {
                wait_for_call.recv().expect("release late model call");
                (owner, ())
            },
        );
        assert!(matches!(timed_out, BoundedNativeCall::Unconfirmed(_)));

        allow_call.send(()).expect("complete late model call");
        wait_for_drop_start
            .recv_timeout(Duration::from_secs(5))
            .expect("late owner destruction must begin");
        assert!(!owner_dropped.load(Ordering::Acquire));
        assert!(matches!(
            run_bounded_native_call(5_u8, Duration::from_secs(1), |owner| (owner, ())),
            BoundedNativeCall::NotStarted {
                owner: 5,
                error: NativeCallLaunchError::CapacityUnavailable,
            }
        ));

        allow_drop
            .send(())
            .expect("allow late owner destruction to finish");
        let deadline = Instant::now()
            .checked_add(Duration::from_secs(1))
            .expect("bounded test deadline");
        while NATIVE_CALL_IN_FLIGHT.load(Ordering::Acquire)
            || !owner_dropped.load(Ordering::Acquire)
        {
            assert!(Instant::now() < deadline, "native helper did not retire");
            thread::park_timeout(Duration::from_millis(1));
        }
        assert!(owner_dropped.load(Ordering::Acquire));

        let completed = run_bounded_native_call(9_u8, Duration::from_secs(5), |owner| (owner, ()));
        assert!(matches!(
            completed,
            BoundedNativeCall::Completed {
                owner: 9,
                result: (),
            }
        ));
    }
}
