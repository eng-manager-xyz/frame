//! Process-lifetime native suspend/resume observations.
//!
//! This crate is the deliberately narrow unsafe boundary for platform power
//! callbacks. Consumers receive only a cloneable monitor and a per-consumer
//! monotonic cursor; native observer identities and callback pointers never
//! cross the public API.

use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use thiserror::Error;

const ASLEEP_BIT: u64 = 1;
const SEQUENCE_SHIFT: u32 = 1;
#[cfg(any(target_os = "macos", target_os = "windows", test))]
const MAX_SEQUENCE: u64 = u64::MAX >> SEQUENCE_SHIFT;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemPowerEvent {
    WillSleep,
    DidWake,
}

impl SystemPowerEvent {
    #[cfg(any(target_os = "macos", target_os = "windows", test))]
    const fn asleep(self) -> bool {
        matches!(self, Self::WillSleep)
    }

    const fn from_asleep(asleep: bool) -> Self {
        if asleep {
            Self::WillSleep
        } else {
            Self::DidWake
        }
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SystemPowerMonitorError {
    #[error("the macOS power observer must be installed on the main thread")]
    MainThreadRequired,
    #[error("the native suspend/resume callback could not be registered")]
    RegistrationFailed,
    #[error("the consumer missed more than one suspend/resume cycle")]
    EventGap,
    #[error("the suspend/resume event sequence exhausted its range")]
    SequenceExhausted,
}

#[derive(Debug, Default)]
struct PowerEventState {
    /// High bits are a monotonic sequence; bit zero is the current sleep state.
    word: AtomicU64,
    sequence_exhausted: AtomicBool,
}

impl PowerEventState {
    #[cfg(any(target_os = "macos", target_os = "windows", test))]
    fn publish(&self, event: SystemPowerEvent) {
        let asleep = event.asleep();
        let result = self
            .word
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |word| {
                let current_asleep = word & ASLEEP_BIT != 0;
                if current_asleep == asleep {
                    return None;
                }
                let sequence = word >> SEQUENCE_SHIFT;
                let next = sequence
                    .checked_add(1)
                    .filter(|next| *next <= MAX_SEQUENCE)?;
                Some((next << SEQUENCE_SHIFT) | u64::from(asleep))
            });
        if let Err(word) = result
            && (word & ASLEEP_BIT != 0) != asleep
        {
            self.sequence_exhausted.store(true, Ordering::Release);
        }
    }

    fn word(&self) -> u64 {
        self.word.load(Ordering::Acquire)
    }
}

/// Process-lifetime registration handle.
///
/// Native observers are intentionally installed once and retained until
/// process exit. Each consumer creates its own cursor, so a backend refresh
/// cannot consume another recording session's event.
#[derive(Debug, Clone)]
pub struct SystemPowerMonitor {
    state: Arc<PowerEventState>,
}

impl SystemPowerMonitor {
    /// Install the native process observer.
    ///
    /// macOS requires this call on the application main thread. Repeated calls
    /// reuse the one process observer and return independent handles.
    pub fn install() -> Result<Self, SystemPowerMonitorError> {
        platform::install().map(|state| Self { state })
    }

    /// Construct an inert monitor for deterministic tests or non-native
    /// compositions.
    #[must_use]
    pub fn detached() -> Self {
        Self {
            state: Arc::new(PowerEventState::default()),
        }
    }

    #[must_use]
    pub fn cursor(&self) -> SystemPowerCursor {
        SystemPowerCursor {
            state: Arc::clone(&self.state),
            observed_word: self.state.word(),
            pending: None,
        }
    }
}

/// Per-consumer, bounded suspend/resume cursor.
///
/// A consumer may lag by one complete sleep/wake cycle. More than two unseen
/// transitions fails closed instead of guessing event order.
#[derive(Debug)]
pub struct SystemPowerCursor {
    state: Arc<PowerEventState>,
    observed_word: u64,
    pending: Option<SystemPowerEvent>,
}

impl SystemPowerCursor {
    #[must_use]
    pub fn is_asleep(&self) -> bool {
        self.state.word() & ASLEEP_BIT != 0
    }

    pub fn poll(&mut self) -> Result<Option<SystemPowerEvent>, SystemPowerMonitorError> {
        if self.state.sequence_exhausted.load(Ordering::Acquire) {
            return Err(SystemPowerMonitorError::SequenceExhausted);
        }
        if let Some(event) = self.pending.take() {
            return Ok(Some(event));
        }

        let current = self.state.word();
        if current == self.observed_word {
            return Ok(None);
        }
        let previous_sequence = self.observed_word >> SEQUENCE_SHIFT;
        let current_sequence = current >> SEQUENCE_SHIFT;
        let Some(delta) = current_sequence.checked_sub(previous_sequence) else {
            return Err(SystemPowerMonitorError::SequenceExhausted);
        };
        if delta == 0 || delta > 2 {
            return Err(SystemPowerMonitorError::EventGap);
        }

        let previous_asleep = self.observed_word & ASLEEP_BIT != 0;
        let current_asleep = current & ASLEEP_BIT != 0;
        let expected_current = if delta == 1 {
            !previous_asleep
        } else {
            previous_asleep
        };
        if current_asleep != expected_current {
            return Err(SystemPowerMonitorError::EventGap);
        }

        self.observed_word = current;
        if delta == 1 {
            return Ok(Some(SystemPowerEvent::from_asleep(current_asleep)));
        }

        self.pending = Some(SystemPowerEvent::from_asleep(current_asleep));
        Ok(Some(SystemPowerEvent::from_asleep(!current_asleep)))
    }
}

fn global_state() -> Arc<PowerEventState> {
    static STATE: OnceLock<Arc<PowerEventState>> = OnceLock::new();
    Arc::clone(STATE.get_or_init(|| Arc::new(PowerEventState::default())))
}

#[cfg(target_os = "macos")]
mod platform {
    use std::sync::{Arc, Mutex, OnceLock};

    use objc2::{
        AnyThread, DeclaredClass, define_class, msg_send,
        rc::{Retained, autoreleasepool},
        runtime::{AnyObject, NSObject},
    };
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSNotification, NSThread, ns_string};

    use super::{PowerEventState, SystemPowerEvent, SystemPowerMonitorError, global_state};

    struct ObserverIvars {
        state: Arc<PowerEventState>,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "FramePowerObserver"]
        #[ivars = ObserverIvars]
        struct FramePowerObserver;

        impl FramePowerObserver {
            #[unsafe(method(handleWillSleep:))]
            fn handle_will_sleep(&self, _notification: &NSNotification) {
                self.ivars().state.publish(SystemPowerEvent::WillSleep);
            }

            #[unsafe(method(handleDidWake:))]
            fn handle_did_wake(&self, _notification: &NSNotification) {
                self.ivars().state.publish(SystemPowerEvent::DidWake);
            }

            #[unsafe(method(handleScreensDidSleep:))]
            fn handle_screens_did_sleep(&self, _notification: &NSNotification) {
                self.ivars().state.publish(SystemPowerEvent::WillSleep);
            }

            #[unsafe(method(handleScreensDidWake:))]
            fn handle_screens_did_wake(&self, _notification: &NSNotification) {
                self.ivars().state.publish(SystemPowerEvent::DidWake);
            }
        }
    );

    impl FramePowerObserver {
        fn new(state: Arc<PowerEventState>) -> Retained<Self> {
            let this = Self::alloc().set_ivars(ObserverIvars { state });
            // SAFETY: `this` is a freshly allocated NSObject subclass with all
            // ivars initialized above; `init` returns the retained instance.
            unsafe { msg_send![super(this), init] }
        }
    }

    static OBSERVER: OnceLock<Mutex<Option<Retained<FramePowerObserver>>>> = OnceLock::new();

    pub(super) fn install() -> Result<Arc<PowerEventState>, SystemPowerMonitorError> {
        let state = global_state();
        let slot = OBSERVER.get_or_init(|| Mutex::new(None));
        let mut observer = slot.lock().unwrap_or_else(|error| error.into_inner());
        if observer.is_some() {
            return Ok(state);
        }
        if !NSThread::isMainThread_class() {
            return Err(SystemPowerMonitorError::MainThreadRequired);
        }

        autoreleasepool(|_| {
            let workspace = NSWorkspace::sharedWorkspace();
            let center = workspace.notificationCenter();
            let installed = FramePowerObserver::new(Arc::clone(&state));
            let installed_object: &AnyObject = &installed;
            // SAFETY: every selector is implemented by FramePowerObserver with
            // the exact one-notification argument ABI. The retained observer is
            // stored process-wide immediately after registration.
            unsafe {
                center.addObserver_selector_name_object(
                    installed_object,
                    objc2::sel!(handleWillSleep:),
                    Some(ns_string!("NSWorkspaceWillSleepNotification")),
                    None,
                );
                center.addObserver_selector_name_object(
                    installed_object,
                    objc2::sel!(handleDidWake:),
                    Some(ns_string!("NSWorkspaceDidWakeNotification")),
                    None,
                );
                center.addObserver_selector_name_object(
                    installed_object,
                    objc2::sel!(handleScreensDidSleep:),
                    Some(ns_string!("NSWorkspaceScreensDidSleepNotification")),
                    None,
                );
                center.addObserver_selector_name_object(
                    installed_object,
                    objc2::sel!(handleScreensDidWake:),
                    Some(ns_string!("NSWorkspaceScreensDidWakeNotification")),
                    None,
                );
            }
            *observer = Some(installed);
        });
        Ok(state)
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::{
        ffi::c_void,
        sync::{Arc, Mutex, OnceLock},
    };

    use windows::Win32::{
        Foundation::{HANDLE, WIN32_ERROR},
        System::Power::{
            DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, PowerRegisterSuspendResumeNotification,
        },
        UI::WindowsAndMessaging::{DEVICE_NOTIFY_CALLBACK, PBT_APMRESUMEAUTOMATIC, PBT_APMSUSPEND},
    };

    use super::{PowerEventState, SystemPowerEvent, SystemPowerMonitorError, global_state};

    struct Registration {
        _handle: *mut c_void,
        _context: Arc<PowerEventState>,
    }

    // SAFETY: the registration owns the callback context for process lifetime;
    // Windows may invoke the callback from a system thread, and PowerEventState
    // contains only an atomic word.
    unsafe impl Send for Registration {}
    // SAFETY: shared access never mutates the handle or context pointer.
    unsafe impl Sync for Registration {}

    static REGISTRATION: OnceLock<Mutex<Option<Registration>>> = OnceLock::new();

    unsafe extern "system" fn power_callback(
        context: *const c_void,
        event_type: u32,
        _setting: *const c_void,
    ) -> u32 {
        if context.is_null() {
            return 0;
        }
        // SAFETY: `context` points to the Arc allocation retained by
        // Registration for the complete native registration lifetime.
        let state = unsafe { &*(context.cast::<PowerEventState>()) };
        match event_type {
            PBT_APMSUSPEND => state.publish(SystemPowerEvent::WillSleep),
            PBT_APMRESUMEAUTOMATIC => state.publish(SystemPowerEvent::DidWake),
            _ => {}
        }
        0
    }

    pub(super) fn install() -> Result<Arc<PowerEventState>, SystemPowerMonitorError> {
        let state = global_state();
        let slot = REGISTRATION.get_or_init(|| Mutex::new(None));
        let mut registration = slot.lock().unwrap_or_else(|error| error.into_inner());
        if registration.is_some() {
            return Ok(state);
        }

        let context = Arc::clone(&state);
        let context_pointer = Arc::as_ptr(&context).cast_mut().cast::<c_void>();
        let parameters = DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
            Callback: Some(power_callback),
            Context: context_pointer,
        };
        let mut raw_handle = std::ptr::null_mut();
        // SAFETY: parameters and output storage live through this synchronous
        // registration call. The Arc context is retained on success.
        let result = unsafe {
            PowerRegisterSuspendResumeNotification(
                DEVICE_NOTIFY_CALLBACK,
                HANDLE(std::ptr::from_ref(&parameters).cast_mut().cast()),
                &mut raw_handle,
            )
        };
        if result != WIN32_ERROR(0) {
            return Err(SystemPowerMonitorError::RegistrationFailed);
        }
        *registration = Some(Registration {
            _handle: raw_handle,
            _context: context,
        });
        Ok(state)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use std::sync::Arc;

    use super::{PowerEventState, SystemPowerMonitorError, global_state};

    pub(super) fn install() -> Result<Arc<PowerEventState>, SystemPowerMonitorError> {
        Ok(global_state())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor() -> SystemPowerMonitor {
        SystemPowerMonitor::detached()
    }

    #[test]
    fn duplicate_native_observations_do_not_advance_the_sequence() {
        let monitor = monitor();
        let mut cursor = monitor.cursor();
        monitor.state.publish(SystemPowerEvent::WillSleep);
        monitor.state.publish(SystemPowerEvent::WillSleep);
        assert_eq!(cursor.poll(), Ok(Some(SystemPowerEvent::WillSleep)));
        assert_eq!(cursor.poll(), Ok(None));
    }

    #[test]
    fn one_complete_unpolled_cycle_is_replayed_in_order() {
        let monitor = monitor();
        let mut cursor = monitor.cursor();
        monitor.state.publish(SystemPowerEvent::WillSleep);
        monitor.state.publish(SystemPowerEvent::DidWake);
        assert_eq!(cursor.poll(), Ok(Some(SystemPowerEvent::WillSleep)));
        assert_eq!(cursor.poll(), Ok(Some(SystemPowerEvent::DidWake)));
        assert_eq!(cursor.poll(), Ok(None));
    }

    #[test]
    fn more_than_one_unpolled_cycle_fails_closed() {
        let monitor = monitor();
        let mut cursor = monitor.cursor();
        for event in [
            SystemPowerEvent::WillSleep,
            SystemPowerEvent::DidWake,
            SystemPowerEvent::WillSleep,
            SystemPowerEvent::DidWake,
        ] {
            monitor.state.publish(event);
        }
        assert_eq!(cursor.poll(), Err(SystemPowerMonitorError::EventGap));
    }

    #[test]
    fn cursor_reports_the_current_sleep_state_without_consuming_events() {
        let monitor = monitor();
        let mut cursor = monitor.cursor();
        monitor.state.publish(SystemPowerEvent::WillSleep);
        assert!(cursor.is_asleep());
        assert_eq!(cursor.poll(), Ok(Some(SystemPowerEvent::WillSleep)));
    }

    #[test]
    fn sequence_exhaustion_is_sticky_and_fails_closed() {
        let monitor = monitor();
        monitor
            .state
            .word
            .store(MAX_SEQUENCE << SEQUENCE_SHIFT, Ordering::Release);
        let mut cursor = monitor.cursor();
        monitor.state.publish(SystemPowerEvent::WillSleep);
        assert_eq!(
            cursor.poll(),
            Err(SystemPowerMonitorError::SequenceExhausted)
        );
        assert_eq!(
            cursor.poll(),
            Err(SystemPowerMonitorError::SequenceExhausted)
        );
    }
}
