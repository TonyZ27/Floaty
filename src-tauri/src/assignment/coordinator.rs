use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;

use crate::assignment::{state, BindingEvent, BindingState, RuntimeState, WindowDescriptor};
use crate::db::window_bindings::{NewWindowBinding, WindowBinding};
use crate::error::{AppError, AppResult};

const TARGET_UNAVAILABLE: &str = "selected target window is unavailable";
const NATIVE_ATTACHMENT_FAILED: &str = "native window-space attachment failed";

#[async_trait]
pub trait BindingRepository: Send + Sync {
    async fn binding(&self, sticky_id: &str) -> AppResult<Option<WindowBinding>>;
    async fn bind(&self, sticky_id: &str, target: &WindowDescriptor) -> AppResult<WindowBinding>;
    async fn set_state(
        &self,
        sticky_id: &str,
        state: BindingState,
        clear_runtime_identity: bool,
    ) -> AppResult<WindowBinding>;
    async fn unbind(&self, sticky_id: &str) -> AppResult<Option<WindowBinding>>;
    async fn manually_hidden(&self, sticky_id: &str) -> AppResult<bool>;
    async fn set_manually_hidden(&self, sticky_id: &str, hidden: bool) -> AppResult<()>;
}

#[async_trait]
pub trait BindingWindowCatalog: Send + Sync {
    async fn contains(&self, pid: i32, window_id: u32) -> AppResult<bool>;
}

#[async_trait]
pub trait NativeWindowRuntime: Send + Sync {
    async fn attach(&self, sticky_id: &str, target_window_id: u32) -> AppResult<()>;
    async fn detach(&self, sticky_id: &str) -> AppResult<()>;
    async fn suspend(&self, sticky_id: &str) -> AppResult<()>;
    async fn resume(&self, sticky_id: &str, target_window_id: u32) -> AppResult<()>;
}

#[async_trait]
pub trait NoteWindowRuntime: Send + Sync {
    async fn ensure_open(&self, sticky_id: &str, keep_hidden: bool) -> AppResult<()>;
    async fn reopen(&self, sticky_id: &str) -> AppResult<()>;
    async fn enter_bound_mode(&self, sticky_id: &str) -> AppResult<()>;
    async fn restore_unbound_pinning(&self, sticky_id: &str) -> AppResult<()>;
    async fn binding_removed(&self, _sticky_id: &str) -> AppResult<()> {
        Ok(())
    }
    async fn binding_changed(&self, _binding: &WindowBinding) -> AppResult<()> {
        Ok(())
    }
}

#[async_trait]
pub trait ObserverRegistration: Send {
    async fn stop(&mut self) -> AppResult<()>;
}

#[async_trait]
pub trait ObserverFactory: Send + Sync {
    async fn observe(
        &self,
        pid: i32,
        window_id: u32,
        callback: Arc<dyn Fn(BindingEvent) + Send + Sync>,
    ) -> AppResult<Box<dyn ObserverRegistration>>;
}

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub attempts: usize,
    pub delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            attempts: 6,
            delay: Duration::from_millis(100),
        }
    }
}

#[derive(Clone)]
pub struct BindingCoordinator {
    inner: Arc<CoordinatorInner>,
}

struct CoordinatorInner {
    repository: Arc<dyn BindingRepository>,
    catalog: Arc<dyn BindingWindowCatalog>,
    native: Arc<dyn NativeWindowRuntime>,
    windows: Arc<dyn NoteWindowRuntime>,
    observers: Arc<dyn ObserverFactory>,
    active_observers: Mutex<HashMap<String, ActiveObserver>>,
    next_observer_generation: AtomicU64,
    runtime_states: Mutex<HashMap<String, RuntimeState>>,
    attached_notes: Mutex<HashSet<String>>,
    event_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    retry: RetryPolicy,
}

struct ActiveObserver {
    generation: u64,
    registration: Box<dyn ObserverRegistration>,
}

impl BindingCoordinator {
    pub fn new(
        repository: Arc<dyn BindingRepository>,
        catalog: Arc<dyn BindingWindowCatalog>,
        native: Arc<dyn NativeWindowRuntime>,
        windows: Arc<dyn NoteWindowRuntime>,
        observers: Arc<dyn ObserverFactory>,
    ) -> Self {
        Self::with_retry_policy(
            repository,
            catalog,
            native,
            windows,
            observers,
            RetryPolicy::default(),
        )
    }

    pub fn with_retry_policy(
        repository: Arc<dyn BindingRepository>,
        catalog: Arc<dyn BindingWindowCatalog>,
        native: Arc<dyn NativeWindowRuntime>,
        windows: Arc<dyn NoteWindowRuntime>,
        observers: Arc<dyn ObserverFactory>,
        retry: RetryPolicy,
    ) -> Self {
        Self {
            inner: Arc::new(CoordinatorInner {
                repository,
                catalog,
                native,
                windows,
                observers,
                active_observers: Mutex::new(HashMap::new()),
                next_observer_generation: AtomicU64::new(1),
                runtime_states: Mutex::new(HashMap::new()),
                attached_notes: Mutex::new(HashSet::new()),
                event_locks: Mutex::new(HashMap::new()),
                retry,
            }),
        }
    }

    pub async fn bind(
        &self,
        sticky_id: &str,
        target: WindowDescriptor,
    ) -> AppResult<WindowBinding> {
        let event_lock = self.event_lock(sticky_id);
        let _event_guard = event_lock.lock().await;
        if !self
            .inner
            .catalog
            .contains(target.pid, target.window_id)
            .await?
        {
            return Err(AppError::Other(TARGET_UNAVAILABLE.into()));
        }

        let previous_binding = self.inner.repository.binding(sticky_id).await?;
        self.stop_observer(sticky_id).await?;
        if previous_binding
            .as_ref()
            .and_then(|binding| binding.runtime_window_id)
            .is_some()
            || self.is_attached(sticky_id)
        {
            if let Err(error) = self.detach(sticky_id).await {
                if let Some(previous) = previous_binding.as_ref() {
                    self.restore_previous_or_degrade(sticky_id, previous)
                        .await?;
                }
                return Err(error);
            }
        }

        let manually_hidden = self.inner.repository.manually_hidden(sticky_id).await?;
        if let Err(error) = self
            .inner
            .windows
            .ensure_open(sticky_id, manually_hidden)
            .await
        {
            if let Some(previous) = previous_binding.as_ref() {
                self.restore_previous_or_degrade(sticky_id, previous)
                    .await?;
            }
            return Err(error);
        }
        if let Err(error) = self.inner.windows.enter_bound_mode(sticky_id).await {
            if let Some(previous) = previous_binding.as_ref() {
                self.restore_previous_or_degrade(sticky_id, previous)
                    .await?;
            } else {
                let _ = self.inner.windows.restore_unbound_pinning(sticky_id).await;
            }
            return Err(error);
        }
        if let Err(error) = self.inner.native.attach(sticky_id, target.window_id).await {
            let _ = self.detach(sticky_id).await;
            if let Some(previous) = previous_binding.as_ref() {
                self.restore_previous_or_degrade(sticky_id, previous)
                    .await?;
            } else {
                let _ = self.inner.windows.restore_unbound_pinning(sticky_id).await;
            }
            return Err(AppError::Other(format!(
                "{NATIVE_ATTACHMENT_FAILED}: {error}"
            )));
        }
        self.mark_attached(sticky_id);
        if manually_hidden {
            if let Err(error) = self.inner.native.suspend(sticky_id).await {
                let _ = self.detach(sticky_id).await;
                if let Some(previous) = previous_binding.as_ref() {
                    self.restore_previous_or_degrade(sticky_id, previous)
                        .await?;
                } else {
                    let _ = self.inner.windows.restore_unbound_pinning(sticky_id).await;
                }
                return Err(error);
            }
        }

        let binding = match self.inner.repository.bind(sticky_id, &target).await {
            Ok(binding) => binding,
            Err(error) => {
                let _ = self.detach(sticky_id).await;
                if let Some(previous) = previous_binding.as_ref() {
                    self.restore_previous_or_degrade(sticky_id, previous)
                        .await?;
                } else {
                    let _ = self.inner.windows.restore_unbound_pinning(sticky_id).await;
                }
                return Err(error);
            }
        };

        if let Err(error) = self
            .install_observer(sticky_id, target.pid, target.window_id)
            .await
        {
            let _ = self.detach(sticky_id).await;
            let unavailable = self
                .inner
                .repository
                .set_state(sticky_id, BindingState::Unavailable, true)
                .await?;
            self.inner.windows.binding_changed(&unavailable).await?;
            return Err(error);
        }

        let runtime_state = if manually_hidden {
            RuntimeState::ManuallyHidden
        } else {
            RuntimeState::Visible
        };
        self.set_runtime_state(sticky_id, runtime_state);
        self.inner.windows.binding_changed(&binding).await?;
        Ok(binding)
    }

    pub async fn unbind(&self, sticky_id: &str) -> AppResult<()> {
        let event_lock = self.event_lock(sticky_id);
        let _event_guard = event_lock.lock().await;
        let binding = self.inner.repository.binding(sticky_id).await?;
        let Some(binding) = binding else {
            self.inner
                .windows
                .restore_unbound_pinning(sticky_id)
                .await?;
            return Ok(());
        };
        self.stop_observer(sticky_id).await?;
        if binding.runtime_window_id.is_some() || self.is_attached(sticky_id) {
            if let Err(error) = self.detach(sticky_id).await {
                self.restore_previous_or_degrade(sticky_id, &binding)
                    .await?;
                return Err(error);
            }
        }
        if let Err(error) = self.inner.repository.unbind(sticky_id).await {
            self.restore_previous_or_degrade(sticky_id, &binding)
                .await?;
            return Err(error);
        }
        let notification_error = self.inner.windows.binding_removed(sticky_id).await.err();
        self.inner
            .runtime_states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(sticky_id);
        self.inner
            .windows
            .restore_unbound_pinning(sticky_id)
            .await?;
        if let Some(error) = notification_error {
            return Err(error);
        }
        Ok(())
    }

    pub async fn prepare_delete(&self, sticky_id: &str) -> AppResult<()> {
        let event_lock = self.event_lock(sticky_id);
        let _event_guard = event_lock.lock().await;
        let binding = self.inner.repository.binding(sticky_id).await?;
        let should_detach = binding
            .as_ref()
            .and_then(|binding| binding.runtime_window_id)
            .is_some()
            || self.is_attached(sticky_id);
        if binding.is_some() {
            self.degrade_binding(sticky_id).await?;
        }
        self.stop_observer(sticky_id).await?;
        if should_detach {
            self.detach(sticky_id).await?;
        }
        self.inner
            .runtime_states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(sticky_id);
        Ok(())
    }

    pub async fn reopen(&self, sticky_id: &str) -> AppResult<WindowBinding> {
        let event_lock = self.event_lock(sticky_id);
        let _event_guard = event_lock.lock().await;
        let binding = self
            .inner
            .repository
            .binding(sticky_id)
            .await?
            .ok_or(AppError::NotFound)?;
        if binding.state != BindingState::Active {
            return Err(AppError::Other(TARGET_UNAVAILABLE.into()));
        }
        let current = self.runtime_state(sticky_id, &binding).await?;
        if current == RuntimeState::SuspendedManuallyHidden {
            if let Err(error) = self.inner.windows.reopen(sticky_id).await {
                let _ = self.inner.native.suspend(sticky_id).await;
                return Err(error);
            }
            if let Err(error) = self
                .inner
                .repository
                .set_manually_hidden(sticky_id, false)
                .await
            {
                let _ = self.inner.native.suspend(sticky_id).await;
                return Err(error);
            }
            self.set_runtime_state(sticky_id, RuntimeState::Suspended);
            return Ok(binding);
        }
        if current != RuntimeState::ManuallyHidden {
            return Ok(binding);
        }
        let pid = active_pid(&binding)?;
        let target_window_id = active_window_id(&binding)?;
        if !self.inner.catalog.contains(pid, target_window_id).await? {
            return Err(AppError::Other(TARGET_UNAVAILABLE.into()));
        }
        if let Err(error) = self.inner.native.attach(sticky_id, target_window_id).await {
            self.transition_unavailable(sticky_id).await?;
            return Err(error);
        }
        self.mark_attached(sticky_id);
        if let Err(error) = self.inner.native.resume(sticky_id, target_window_id).await {
            self.transition_unavailable(sticky_id).await?;
            return Err(error);
        }
        if let Err(error) = self.inner.windows.reopen(sticky_id).await {
            let _ = self.inner.native.suspend(sticky_id).await;
            return Err(error);
        }
        if let Err(error) = self
            .inner
            .repository
            .set_manually_hidden(sticky_id, false)
            .await
        {
            let _ = self.inner.native.suspend(sticky_id).await;
            return Err(error);
        }
        self.set_runtime_state(sticky_id, RuntimeState::Visible);
        Ok(binding)
    }

    pub async fn handle(&self, sticky_id: &str, event: BindingEvent) -> AppResult<()> {
        let event_lock = self.event_lock(sticky_id);
        let _event_guard = event_lock.lock().await;
        self.handle_locked(sticky_id, event).await
    }

    async fn handle_locked(&self, sticky_id: &str, event: BindingEvent) -> AppResult<()> {
        let binding = self
            .inner
            .repository
            .binding(sticky_id)
            .await?
            .ok_or(AppError::NotFound)?;
        let current = self.runtime_state(sticky_id, &binding).await?;

        match event {
            BindingEvent::TargetMinimized => {
                if matches!(
                    current,
                    RuntimeState::Visible | RuntimeState::ManuallyHidden
                ) {
                    self.inner.native.suspend(sticky_id).await?;
                }
            }
            BindingEvent::TargetRestored => {
                if current == RuntimeState::Suspended
                    && !self.inner.repository.manually_hidden(sticky_id).await?
                {
                    self.inner
                        .native
                        .resume(sticky_id, active_window_id(&binding)?)
                        .await?;
                }
            }
            BindingEvent::TargetClosed => {
                let updated = self
                    .inner
                    .repository
                    .set_state(sticky_id, BindingState::TargetClosed, true)
                    .await?;
                self.set_runtime_state(sticky_id, RuntimeState::TargetClosed);
                let cleanup_error = self.stop_observer(sticky_id).await.err().or(self
                    .inner
                    .native
                    .suspend(sticky_id)
                    .await
                    .err());
                self.inner.windows.binding_changed(&updated).await?;
                if let Some(error) = cleanup_error {
                    return Err(error);
                }
            }
            BindingEvent::TargetSpaceChanged => {
                if matches!(
                    current,
                    RuntimeState::Suspended | RuntimeState::SuspendedManuallyHidden
                ) {
                    return Ok(());
                }
                self.reattach_exact_target(sticky_id, &binding).await?;
                return Ok(());
            }
            BindingEvent::PermissionRevoked => {
                let updated = self
                    .inner
                    .repository
                    .set_state(sticky_id, BindingState::Unavailable, true)
                    .await?;
                self.set_runtime_state(sticky_id, RuntimeState::Unavailable);
                let cleanup_error = self.stop_observer(sticky_id).await.err().or(self
                    .inner
                    .native
                    .suspend(sticky_id)
                    .await
                    .err());
                self.inner.windows.binding_changed(&updated).await?;
                if let Some(error) = cleanup_error {
                    return Err(error);
                }
            }
            BindingEvent::UserHidNote
            | BindingEvent::UserOpenedNote
            | BindingEvent::SimilarWindowOpened => {}
        }

        self.set_runtime_state(sticky_id, state::reduce(current, event));
        Ok(())
    }

    pub fn runtime_state_for(&self, sticky_id: &str) -> Option<RuntimeState> {
        self.inner
            .runtime_states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(sticky_id)
            .copied()
    }

    async fn reattach_exact_target(
        &self,
        sticky_id: &str,
        binding: &WindowBinding,
    ) -> AppResult<()> {
        let pid = active_pid(binding)?;
        let window_id = active_window_id(binding)?;
        let attempts = self.inner.retry.attempts.max(1);

        for attempt in 0..attempts {
            let exact_target_available = self.inner.catalog.contains(pid, window_id).await;
            if matches!(exact_target_available, Ok(true))
                && self.inner.native.attach(sticky_id, window_id).await.is_ok()
            {
                self.mark_attached(sticky_id);
                return Ok(());
            }
            if attempt + 1 < attempts && !self.inner.retry.delay.is_zero() {
                tokio::time::sleep(self.inner.retry.delay).await;
            }
        }

        let updated = self
            .inner
            .repository
            .set_state(sticky_id, BindingState::Unavailable, true)
            .await?;
        self.set_runtime_state(sticky_id, RuntimeState::Unavailable);
        let cleanup_error = self.stop_observer(sticky_id).await.err().or(self
            .inner
            .native
            .suspend(sticky_id)
            .await
            .err());
        self.inner.windows.binding_changed(&updated).await?;
        Err(cleanup_error.unwrap_or_else(|| AppError::Other(NATIVE_ATTACHMENT_FAILED.into())))
    }

    async fn install_observer(&self, sticky_id: &str, pid: i32, window_id: u32) -> AppResult<()> {
        let generation = self
            .inner
            .next_observer_generation
            .fetch_add(1, Ordering::Relaxed);
        let weak: Weak<CoordinatorInner> = Arc::downgrade(&self.inner);
        let sticky_id_for_callback = sticky_id.to_owned();
        let callback = Arc::new(move |event| {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            let coordinator = BindingCoordinator { inner };
            let sticky_id = sticky_id_for_callback.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = coordinator
                    .handle_observed(&sticky_id, generation, event)
                    .await
                {
                    eprintln!("[floaty] binding event failed for {}: {}", sticky_id, error);
                }
            });
        });
        let observer = self
            .inner
            .observers
            .observe(pid, window_id, callback)
            .await?;
        self.inner
            .active_observers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                sticky_id.to_owned(),
                ActiveObserver {
                    generation,
                    registration: observer,
                },
            );
        Ok(())
    }

    async fn handle_observed(
        &self,
        sticky_id: &str,
        generation: u64,
        event: BindingEvent,
    ) -> AppResult<()> {
        let event_lock = self.event_lock(sticky_id);
        let _event_guard = event_lock.lock().await;
        let is_current = self
            .inner
            .active_observers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(sticky_id)
            .map(|observer| observer.generation == generation)
            .unwrap_or(false);
        if !is_current {
            return Ok(());
        }
        self.handle_locked(sticky_id, event).await
    }

    async fn stop_observer(&self, sticky_id: &str) -> AppResult<()> {
        let observer = self
            .inner
            .active_observers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(sticky_id);
        if let Some(mut observer) = observer {
            if let Err(error) = observer.registration.stop().await {
                self.inner
                    .active_observers
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .insert(sticky_id.to_owned(), observer);
                return Err(error);
            }
        }
        Ok(())
    }

    async fn runtime_state(
        &self,
        sticky_id: &str,
        binding: &WindowBinding,
    ) -> AppResult<RuntimeState> {
        if let Some(state) = self.runtime_state_for(sticky_id) {
            return Ok(state);
        }
        let state = match binding.state {
            BindingState::Active => {
                if self.inner.repository.manually_hidden(sticky_id).await? {
                    RuntimeState::ManuallyHidden
                } else {
                    RuntimeState::Visible
                }
            }
            BindingState::TargetClosed => RuntimeState::TargetClosed,
            BindingState::ReconnectRequired => RuntimeState::ReconnectRequired,
            BindingState::Unavailable => RuntimeState::Unavailable,
        };
        self.set_runtime_state(sticky_id, state);
        Ok(state)
    }

    fn set_runtime_state(&self, sticky_id: &str, state: RuntimeState) {
        self.inner
            .runtime_states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(sticky_id.to_owned(), state);
    }

    fn event_lock(&self, sticky_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.inner
            .event_locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(sticky_id.to_owned())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    fn mark_attached(&self, sticky_id: &str) {
        self.inner
            .attached_notes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(sticky_id.to_owned());
    }

    fn is_attached(&self, sticky_id: &str) -> bool {
        self.inner
            .attached_notes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(sticky_id)
    }

    async fn detach(&self, sticky_id: &str) -> AppResult<()> {
        self.inner.native.detach(sticky_id).await?;
        self.inner
            .attached_notes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(sticky_id);
        Ok(())
    }

    async fn degrade_binding(&self, sticky_id: &str) -> AppResult<()> {
        let updated = self
            .inner
            .repository
            .set_state(sticky_id, BindingState::Unavailable, true)
            .await?;
        self.set_runtime_state(sticky_id, RuntimeState::Unavailable);
        self.inner.windows.binding_changed(&updated).await
    }

    async fn restore_previous_or_degrade(
        &self,
        sticky_id: &str,
        binding: &WindowBinding,
    ) -> AppResult<()> {
        if self.restore_previous(sticky_id, binding).await.is_ok() {
            let state = if self.inner.repository.manually_hidden(sticky_id).await? {
                RuntimeState::ManuallyHidden
            } else {
                RuntimeState::Visible
            };
            self.set_runtime_state(sticky_id, state);
            return Ok(());
        }
        self.transition_unavailable(sticky_id).await
    }

    async fn restore_previous(&self, sticky_id: &str, binding: &WindowBinding) -> AppResult<()> {
        let pid = active_pid(binding)?;
        let window_id = active_window_id(binding)?;
        if !self.inner.catalog.contains(pid, window_id).await? {
            return Err(AppError::Other(TARGET_UNAVAILABLE.into()));
        }
        self.inner.native.attach(sticky_id, window_id).await?;
        self.mark_attached(sticky_id);
        self.install_observer(sticky_id, pid, window_id).await
    }

    async fn transition_unavailable(&self, sticky_id: &str) -> AppResult<()> {
        let updated = self
            .inner
            .repository
            .set_state(sticky_id, BindingState::Unavailable, true)
            .await?;
        self.set_runtime_state(sticky_id, RuntimeState::Unavailable);
        let cleanup_error = self.stop_observer(sticky_id).await.err().or(self
            .inner
            .native
            .suspend(sticky_id)
            .await
            .err());
        self.inner.windows.binding_changed(&updated).await?;
        if let Some(error) = cleanup_error {
            return Err(error);
        }
        Ok(())
    }
}

fn active_pid(binding: &WindowBinding) -> AppResult<i32> {
    binding
        .runtime_pid
        .and_then(|pid| i32::try_from(pid).ok())
        .filter(|pid| *pid > 0)
        .ok_or_else(|| AppError::Other(TARGET_UNAVAILABLE.into()))
}

fn active_window_id(binding: &WindowBinding) -> AppResult<u32> {
    binding
        .runtime_window_id
        .and_then(|window_id| u32::try_from(window_id).ok())
        .filter(|window_id| *window_id != 0)
        .ok_or_else(|| AppError::Other(TARGET_UNAVAILABLE.into()))
}

pub struct DbBindingRepository {
    db: crate::db::Db,
}

impl DbBindingRepository {
    pub fn new(db: crate::db::Db) -> Self {
        Self { db }
    }
}

#[async_trait]
impl BindingRepository for DbBindingRepository {
    async fn binding(&self, sticky_id: &str) -> AppResult<Option<WindowBinding>> {
        crate::db::window_bindings::get(&self.db, sticky_id).await
    }

    async fn bind(&self, sticky_id: &str, target: &WindowDescriptor) -> AppResult<WindowBinding> {
        crate::db::window_bindings::bind(
            &self.db,
            NewWindowBinding {
                sticky_id: sticky_id.to_owned(),
                app_bundle_id: target.app_bundle_id.clone(),
                app_name: target.app_name.clone(),
                window_title: target.window_title.clone(),
                runtime_pid: i64::from(target.pid),
                runtime_window_id: i64::from(target.window_id),
            },
        )
        .await
    }

    async fn set_state(
        &self,
        sticky_id: &str,
        state: BindingState,
        clear_runtime_identity: bool,
    ) -> AppResult<WindowBinding> {
        if clear_runtime_identity {
            crate::db::window_bindings::set_state_and_clear_runtime(&self.db, sticky_id, state)
                .await
        } else {
            crate::db::window_bindings::set_state(&self.db, sticky_id, state).await
        }
    }

    async fn unbind(&self, sticky_id: &str) -> AppResult<Option<WindowBinding>> {
        crate::db::window_bindings::unbind(&self.db, sticky_id).await
    }

    async fn manually_hidden(&self, sticky_id: &str) -> AppResult<bool> {
        Ok(crate::db::stickies::get(&self.db, sticky_id).await?.hidden == 1)
    }

    async fn set_manually_hidden(&self, sticky_id: &str, hidden: bool) -> AppResult<()> {
        crate::db::stickies::update(
            &self.db,
            sticky_id,
            crate::db::stickies::StickyPatch {
                hidden: Some(i64::from(hidden)),
                ..Default::default()
            },
        )
        .await?;
        Ok(())
    }
}

pub struct MainThreadWindowCatalog {
    app: tauri::AppHandle,
    catalog: Arc<dyn crate::assignment::WindowCatalog>,
}

impl MainThreadWindowCatalog {
    pub fn new(app: tauri::AppHandle, catalog: Arc<dyn crate::assignment::WindowCatalog>) -> Self {
        Self { app, catalog }
    }

    pub async fn list_eligible(&self) -> AppResult<Vec<WindowDescriptor>> {
        run_on_main_thread(&self.app, {
            let catalog = self.catalog.clone();
            move || catalog.list_eligible()
        })
        .await
    }
}

#[async_trait]
impl BindingWindowCatalog for MainThreadWindowCatalog {
    async fn contains(&self, pid: i32, window_id: u32) -> AppResult<bool> {
        run_on_main_thread(&self.app, {
            let catalog = self.catalog.clone();
            move || catalog.contains(pid, window_id)
        })
        .await
    }
}

#[cfg(target_os = "macos")]
pub struct TauriNativeWindowRuntime {
    app: tauri::AppHandle,
    spaces: Arc<dyn crate::platform::macos::window_spaces::WindowSpaceBackend>,
}

#[cfg(target_os = "macos")]
impl TauriNativeWindowRuntime {
    pub fn new(
        app: tauri::AppHandle,
        spaces: Arc<dyn crate::platform::macos::window_spaces::WindowSpaceBackend>,
    ) -> Self {
        Self { app, spaces }
    }
}

#[cfg(target_os = "macos")]
#[async_trait]
impl NativeWindowRuntime for TauriNativeWindowRuntime {
    async fn attach(&self, sticky_id: &str, target_window_id: u32) -> AppResult<()> {
        let sticky_id = sticky_id.to_owned();
        run_on_main_thread(&self.app, {
            let app = self.app.clone();
            let spaces = self.spaces.clone();
            move || {
                use tauri::Manager;
                let window = app
                    .get_webview_window(&crate::windows::label(&sticky_id))
                    .ok_or(AppError::NotFound)?;
                let note_window_id =
                    crate::platform::macos::window_spaces::note_window_id(&window)?;
                spaces.attach(target_window_id, note_window_id).map(|_| ())
            }
        })
        .await
    }

    async fn detach(&self, sticky_id: &str) -> AppResult<()> {
        let sticky_id = sticky_id.to_owned();
        run_on_main_thread(&self.app, {
            let app = self.app.clone();
            let spaces = self.spaces.clone();
            move || {
                use tauri::Manager;
                let window = app
                    .get_webview_window(&crate::windows::label(&sticky_id))
                    .ok_or(AppError::NotFound)?;
                let note_window_id =
                    crate::platform::macos::window_spaces::note_window_id(&window)?;
                spaces.detach(note_window_id)
            }
        })
        .await
    }

    async fn suspend(&self, sticky_id: &str) -> AppResult<()> {
        let sticky_id = sticky_id.to_owned();
        run_on_main_thread(&self.app, {
            let app = self.app.clone();
            let spaces = self.spaces.clone();
            move || crate::windows::suspend_for_binding(&app, &sticky_id, spaces.as_ref())
        })
        .await
    }

    async fn resume(&self, sticky_id: &str, target_window_id: u32) -> AppResult<()> {
        let sticky_id = sticky_id.to_owned();
        run_on_main_thread(&self.app, {
            let app = self.app.clone();
            let spaces = self.spaces.clone();
            move || {
                crate::windows::resume_for_binding(
                    &app,
                    &sticky_id,
                    target_window_id,
                    spaces.as_ref(),
                )
            }
        })
        .await
    }
}

pub struct TauriNoteWindowRuntime {
    app: tauri::AppHandle,
    db: crate::db::Db,
}

impl TauriNoteWindowRuntime {
    pub fn new(app: tauri::AppHandle, db: crate::db::Db) -> Self {
        Self { app, db }
    }
}

#[async_trait]
impl NoteWindowRuntime for TauriNoteWindowRuntime {
    async fn ensure_open(&self, sticky_id: &str, keep_hidden: bool) -> AppResult<()> {
        use tauri::Manager;
        let sticky = crate::db::stickies::get(&self.db, sticky_id).await?;
        if keep_hidden {
            crate::windows::open_for_binding(&self.app, &sticky).await?;
        } else if self
            .app
            .get_webview_window(&crate::windows::label(sticky_id))
            .is_none()
        {
            crate::windows::open(&self.app, &sticky).await?;
        }
        Ok(())
    }

    async fn reopen(&self, sticky_id: &str) -> AppResult<()> {
        crate::windows::reveal_for_binding(&self.app, sticky_id, &self.db).await?;
        Ok(())
    }

    async fn enter_bound_mode(&self, sticky_id: &str) -> AppResult<()> {
        use tauri::Manager;
        let window = self
            .app
            .get_webview_window(&crate::windows::label(sticky_id))
            .ok_or(AppError::NotFound)?;
        window
            .set_always_on_top(false)
            .map_err(|error| AppError::Other(format!("disable global pinning: {error}")))
    }

    async fn restore_unbound_pinning(&self, sticky_id: &str) -> AppResult<()> {
        use tauri::Manager;
        let sticky = crate::db::stickies::get(&self.db, sticky_id).await?;
        if let Some(window) = self
            .app
            .get_webview_window(&crate::windows::label(sticky_id))
        {
            window
                .set_always_on_top(sticky.pinned == 1)
                .map_err(|error| AppError::Other(format!("restore global pinning: {error}")))?;
        }
        Ok(())
    }

    async fn binding_removed(&self, sticky_id: &str) -> AppResult<()> {
        use tauri::Emitter;
        if let Err(error) = self.app.emit("binding-changed", sticky_id) {
            eprintln!(
                "[floaty] emit binding-changed failed for {}: {}",
                sticky_id, error
            );
        }
        crate::tray::refresh_menu(&self.app).await;
        Ok(())
    }

    async fn binding_changed(&self, binding: &WindowBinding) -> AppResult<()> {
        use tauri::Emitter;
        if let Err(error) = self.app.emit("binding-changed", &binding.sticky_id) {
            eprintln!(
                "[floaty] emit binding-changed failed for {}: {}",
                binding.sticky_id, error
            );
        }
        crate::tray::refresh_menu(&self.app).await;
        Ok(())
    }
}

async fn run_on_main_thread<T, F>(app: &tauri::AppHandle, operation: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> AppResult<T> + Send + 'static,
{
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = sender.send(operation());
    })
    .map_err(|error| AppError::Other(format!("main-thread dispatch failed: {error}")))?;
    receiver
        .await
        .map_err(|_| AppError::Other("main-thread operation was cancelled".into()))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};

    const STICKY_ID: &str = "sticky";
    const TARGET_PID: i32 = 42;
    const TARGET_WINDOW_ID: u32 = 101;

    #[derive(Default)]
    struct FakeRepository {
        binding: Mutex<Option<WindowBinding>>,
        hidden: AtomicBool,
        fail_unbind: AtomicBool,
        fail_set_state: AtomicBool,
        fail_set_hidden: AtomicBool,
    }

    impl FakeRepository {
        fn active(hidden: bool) -> Self {
            Self {
                binding: Mutex::new(Some(test_binding())),
                hidden: AtomicBool::new(hidden),
                fail_unbind: AtomicBool::new(false),
                fail_set_state: AtomicBool::new(false),
                fail_set_hidden: AtomicBool::new(false),
            }
        }
    }

    #[async_trait]
    impl BindingRepository for FakeRepository {
        async fn binding(&self, _sticky_id: &str) -> AppResult<Option<WindowBinding>> {
            Ok(self.binding.lock().unwrap().clone())
        }

        async fn bind(
            &self,
            _sticky_id: &str,
            target: &WindowDescriptor,
        ) -> AppResult<WindowBinding> {
            let mut binding = test_binding();
            binding.runtime_pid = Some(i64::from(target.pid));
            binding.runtime_window_id = Some(i64::from(target.window_id));
            *self.binding.lock().unwrap() = Some(binding.clone());
            Ok(binding)
        }

        async fn set_state(
            &self,
            _sticky_id: &str,
            state: BindingState,
            clear_runtime_identity: bool,
        ) -> AppResult<WindowBinding> {
            if self.fail_set_state.swap(false, Ordering::SeqCst) {
                return Err(AppError::Other("set state failed".into()));
            }
            let mut guard = self.binding.lock().unwrap();
            let binding = guard.as_mut().ok_or(AppError::NotFound)?;
            binding.state = state;
            if clear_runtime_identity {
                binding.runtime_pid = None;
                binding.runtime_window_id = None;
            }
            Ok(binding.clone())
        }

        async fn unbind(&self, _sticky_id: &str) -> AppResult<Option<WindowBinding>> {
            if self.fail_unbind.swap(false, Ordering::SeqCst) {
                return Err(AppError::Other("unbind failed".into()));
            }
            Ok(self.binding.lock().unwrap().take())
        }

        async fn manually_hidden(&self, _sticky_id: &str) -> AppResult<bool> {
            Ok(self.hidden.load(Ordering::SeqCst))
        }

        async fn set_manually_hidden(&self, _sticky_id: &str, hidden: bool) -> AppResult<()> {
            if self.fail_set_hidden.swap(false, Ordering::SeqCst) {
                return Err(AppError::Other("set hidden failed".into()));
            }
            self.hidden.store(hidden, Ordering::SeqCst);
            Ok(())
        }
    }

    struct FakeCatalog {
        results: Mutex<VecDeque<AppResult<bool>>>,
        identities: Mutex<Vec<(i32, u32)>>,
    }

    impl FakeCatalog {
        fn available() -> Self {
            Self {
                results: Mutex::new(VecDeque::from([Ok(true)])),
                identities: Mutex::new(Vec::new()),
            }
        }

        fn with_results(results: Vec<AppResult<bool>>) -> Self {
            Self {
                results: Mutex::new(results.into()),
                identities: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl BindingWindowCatalog for FakeCatalog {
        async fn contains(&self, pid: i32, window_id: u32) -> AppResult<bool> {
            self.identities.lock().unwrap().push((pid, window_id));
            self.results.lock().unwrap().pop_front().unwrap_or(Ok(true))
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum NativeCall {
        Attach(u32),
        Detach,
        Suspend,
        Resume(u32),
    }

    #[derive(Default)]
    struct FakeNative {
        calls: Mutex<Vec<NativeCall>>,
        attach_results: Mutex<VecDeque<AppResult<()>>>,
        order_log: Arc<Mutex<Vec<&'static str>>>,
        fail_suspend: AtomicBool,
        fail_detach: AtomicBool,
    }

    impl FakeNative {
        fn with_order_log(order_log: Arc<Mutex<Vec<&'static str>>>) -> Self {
            Self {
                order_log,
                ..Default::default()
            }
        }
    }

    #[async_trait]
    impl NativeWindowRuntime for FakeNative {
        async fn attach(&self, _sticky_id: &str, target_window_id: u32) -> AppResult<()> {
            self.calls
                .lock()
                .unwrap()
                .push(NativeCall::Attach(target_window_id));
            self.attach_results
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok(()))
        }

        async fn detach(&self, _sticky_id: &str) -> AppResult<()> {
            self.order_log.lock().unwrap().push("detach");
            self.calls.lock().unwrap().push(NativeCall::Detach);
            if self.fail_detach.swap(false, Ordering::SeqCst) {
                Err(AppError::Other("detach failed".into()))
            } else {
                Ok(())
            }
        }

        async fn suspend(&self, _sticky_id: &str) -> AppResult<()> {
            self.calls.lock().unwrap().push(NativeCall::Suspend);
            if self.fail_suspend.swap(false, Ordering::SeqCst) {
                Err(AppError::Other("order out failed".into()))
            } else {
                Ok(())
            }
        }

        async fn resume(&self, _sticky_id: &str, target_window_id: u32) -> AppResult<()> {
            self.calls
                .lock()
                .unwrap()
                .push(NativeCall::Resume(target_window_id));
            Ok(())
        }
    }

    struct FakeWindows {
        geometry: Mutex<(i64, i64, i64, i64)>,
        ensure_keep_hidden: Mutex<Vec<bool>>,
        reopen_count: Mutex<usize>,
        restore_pin_count: Mutex<usize>,
        binding_removed_count: Mutex<usize>,
        fail_reopen: AtomicBool,
        fail_restore_pinning: AtomicBool,
    }

    impl Default for FakeWindows {
        fn default() -> Self {
            Self {
                geometry: Mutex::new((80, 80, 320, 420)),
                ensure_keep_hidden: Mutex::new(Vec::new()),
                reopen_count: Mutex::new(0),
                restore_pin_count: Mutex::new(0),
                binding_removed_count: Mutex::new(0),
                fail_reopen: AtomicBool::new(false),
                fail_restore_pinning: AtomicBool::new(false),
            }
        }
    }

    #[async_trait]
    impl NoteWindowRuntime for FakeWindows {
        async fn ensure_open(&self, _sticky_id: &str, keep_hidden: bool) -> AppResult<()> {
            self.ensure_keep_hidden.lock().unwrap().push(keep_hidden);
            Ok(())
        }

        async fn reopen(&self, _sticky_id: &str) -> AppResult<()> {
            *self.reopen_count.lock().unwrap() += 1;
            if self.fail_reopen.swap(false, Ordering::SeqCst) {
                Err(AppError::Other("reveal failed".into()))
            } else {
                Ok(())
            }
        }

        async fn enter_bound_mode(&self, _sticky_id: &str) -> AppResult<()> {
            Ok(())
        }

        async fn restore_unbound_pinning(&self, _sticky_id: &str) -> AppResult<()> {
            *self.restore_pin_count.lock().unwrap() += 1;
            if self.fail_restore_pinning.swap(false, Ordering::SeqCst) {
                Err(AppError::Other("restore pinning failed".into()))
            } else {
                Ok(())
            }
        }

        async fn binding_removed(&self, _sticky_id: &str) -> AppResult<()> {
            *self.binding_removed_count.lock().unwrap() += 1;
            Ok(())
        }
    }

    struct FakeObserver {
        stopped: bool,
        order_log: Arc<Mutex<Vec<&'static str>>>,
        fail_stop: Arc<AtomicBool>,
    }

    #[async_trait]
    impl ObserverRegistration for FakeObserver {
        async fn stop(&mut self) -> AppResult<()> {
            if self.fail_stop.swap(false, Ordering::SeqCst) {
                return Err(AppError::Other("observer stop failed".into()));
            }
            if !self.stopped {
                self.order_log.lock().unwrap().push("observer_stop");
                self.stopped = true;
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeObservers {
        order_log: Arc<Mutex<Vec<&'static str>>>,
        fail_stop: Arc<AtomicBool>,
    }

    #[async_trait]
    impl ObserverFactory for FakeObservers {
        async fn observe(
            &self,
            _pid: i32,
            _window_id: u32,
            _callback: Arc<dyn Fn(BindingEvent) + Send + Sync>,
        ) -> AppResult<Box<dyn ObserverRegistration>> {
            Ok(Box::new(FakeObserver {
                stopped: false,
                order_log: self.order_log.clone(),
                fail_stop: self.fail_stop.clone(),
            }))
        }
    }

    struct Harness {
        coordinator: BindingCoordinator,
        repository: Arc<FakeRepository>,
        catalog: Arc<FakeCatalog>,
        native: Arc<FakeNative>,
        windows: Arc<FakeWindows>,
        observers: Arc<FakeObservers>,
    }

    impl Harness {
        fn active(hidden: bool) -> Self {
            Self::with_catalog(hidden, Arc::new(FakeCatalog::available()))
        }

        fn with_catalog(hidden: bool, catalog: Arc<FakeCatalog>) -> Self {
            let repository = Arc::new(FakeRepository::active(hidden));
            let windows = Arc::new(FakeWindows::default());
            let observers = Arc::new(FakeObservers::default());
            let native = Arc::new(FakeNative::with_order_log(observers.order_log.clone()));
            let coordinator = BindingCoordinator::with_retry_policy(
                repository.clone(),
                catalog.clone(),
                native.clone(),
                windows.clone(),
                observers.clone(),
                RetryPolicy {
                    attempts: 3,
                    delay: Duration::ZERO,
                },
            );
            Self {
                coordinator,
                repository,
                catalog,
                native,
                windows,
                observers,
            }
        }

        async fn install_observer(&self) {
            self.coordinator
                .install_observer(STICKY_ID, TARGET_PID, TARGET_WINDOW_ID)
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn minimizing_target_suspends_without_marking_note_hidden() {
        let harness = Harness::active(false);

        harness
            .coordinator
            .handle(STICKY_ID, BindingEvent::TargetMinimized)
            .await
            .unwrap();

        assert_eq!(
            *harness.native.calls.lock().unwrap(),
            vec![NativeCall::Suspend]
        );
        assert!(!harness.repository.hidden.load(Ordering::SeqCst));
        assert_eq!(
            harness.coordinator.runtime_state_for(STICKY_ID),
            Some(RuntimeState::Suspended)
        );
    }

    #[tokio::test]
    async fn manual_hide_prevents_restore() {
        let harness = Harness::active(true);
        harness
            .coordinator
            .set_runtime_state(STICKY_ID, RuntimeState::SuspendedManuallyHidden);

        harness
            .coordinator
            .handle(STICKY_ID, BindingEvent::TargetRestored)
            .await
            .unwrap();

        assert!(harness.native.calls.lock().unwrap().is_empty());
        assert_eq!(
            harness.coordinator.runtime_state_for(STICKY_ID),
            Some(RuntimeState::ManuallyHidden)
        );
    }

    #[tokio::test]
    async fn closing_target_preserves_binding_and_clears_runtime_ids() {
        let harness = Harness::active(false);
        harness.install_observer().await;

        harness
            .coordinator
            .handle(STICKY_ID, BindingEvent::TargetClosed)
            .await
            .unwrap();

        let binding = harness
            .repository
            .binding(STICKY_ID)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(binding.state, BindingState::TargetClosed);
        assert_eq!(binding.runtime_pid, None);
        assert_eq!(binding.runtime_window_id, None);
        assert_eq!(
            harness.coordinator.runtime_state_for(STICKY_ID),
            Some(RuntimeState::TargetClosed)
        );
        assert_eq!(
            *harness.observers.order_log.lock().unwrap(),
            vec!["observer_stop"]
        );
    }

    #[tokio::test]
    async fn target_space_change_retries_same_identity_without_changing_note_geometry() {
        let catalog = Arc::new(FakeCatalog::with_results(vec![
            Err(AppError::Other("transient".into())),
            Ok(true),
        ]));
        let harness = Harness::with_catalog(false, catalog);
        let original_geometry = *harness.windows.geometry.lock().unwrap();

        harness
            .coordinator
            .handle(STICKY_ID, BindingEvent::TargetSpaceChanged)
            .await
            .unwrap();

        assert_eq!(
            *harness.catalog.identities.lock().unwrap(),
            vec![
                (TARGET_PID, TARGET_WINDOW_ID),
                (TARGET_PID, TARGET_WINDOW_ID)
            ]
        );
        assert_eq!(
            *harness.native.calls.lock().unwrap(),
            vec![NativeCall::Attach(TARGET_WINDOW_ID)]
        );
        assert_eq!(*harness.windows.geometry.lock().unwrap(), original_geometry);
    }

    #[tokio::test]
    async fn native_attach_transient_failure_is_retried_for_same_window() {
        let harness = Harness::active(false);
        *harness.native.attach_results.lock().unwrap() =
            VecDeque::from([Err(AppError::Other("space unavailable".into())), Ok(())]);

        harness
            .coordinator
            .handle(STICKY_ID, BindingEvent::TargetSpaceChanged)
            .await
            .unwrap();

        assert_eq!(
            *harness.native.calls.lock().unwrap(),
            vec![
                NativeCall::Attach(TARGET_WINDOW_ID),
                NativeCall::Attach(TARGET_WINDOW_ID)
            ]
        );
    }

    #[tokio::test]
    async fn exhausted_space_retry_is_terminal_until_user_rebinds() {
        let catalog = Arc::new(FakeCatalog::with_results(vec![
            Ok(false),
            Ok(false),
            Ok(false),
        ]));
        let harness = Harness::with_catalog(false, catalog);
        harness.install_observer().await;

        assert!(harness
            .coordinator
            .handle(STICKY_ID, BindingEvent::TargetSpaceChanged)
            .await
            .is_err());

        let binding = harness
            .repository
            .binding(STICKY_ID)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(binding.state, BindingState::Unavailable);
        assert_eq!(binding.runtime_pid, None);
        assert_eq!(binding.runtime_window_id, None);
        assert!(!harness
            .coordinator
            .inner
            .active_observers
            .lock()
            .unwrap()
            .contains_key(STICKY_ID));

        harness
            .coordinator
            .handle(STICKY_ID, BindingEvent::SimilarWindowOpened)
            .await
            .unwrap();
        assert_eq!(
            harness.coordinator.runtime_state_for(STICKY_ID),
            Some(RuntimeState::Unavailable)
        );
        assert_eq!(
            *harness.native.calls.lock().unwrap(),
            vec![NativeCall::Suspend]
        );
    }

    #[tokio::test]
    async fn exhausted_space_retry_keeps_observer_when_terminal_write_fails() {
        let catalog = Arc::new(FakeCatalog::with_results(vec![
            Ok(false),
            Ok(false),
            Ok(false),
        ]));
        let harness = Harness::with_catalog(false, catalog);
        harness.install_observer().await;
        harness
            .repository
            .fail_set_state
            .store(true, Ordering::SeqCst);

        assert!(harness
            .coordinator
            .handle(STICKY_ID, BindingEvent::TargetSpaceChanged)
            .await
            .is_err());

        let binding = harness
            .repository
            .binding(STICKY_ID)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(binding.state, BindingState::Active);
        assert_eq!(binding.runtime_window_id, Some(i64::from(TARGET_WINDOW_ID)));
        assert!(harness
            .coordinator
            .inner
            .active_observers
            .lock()
            .unwrap()
            .contains_key(STICKY_ID));
        assert!(harness.native.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn resize_signal_while_minimized_does_not_attempt_attachment() {
        let harness = Harness::active(false);
        harness
            .coordinator
            .handle(STICKY_ID, BindingEvent::TargetMinimized)
            .await
            .unwrap();
        harness.native.calls.lock().unwrap().clear();

        harness
            .coordinator
            .handle(STICKY_ID, BindingEvent::TargetSpaceChanged)
            .await
            .unwrap();

        assert!(harness.catalog.identities.lock().unwrap().is_empty());
        assert!(harness.native.calls.lock().unwrap().is_empty());
        assert_eq!(
            harness.coordinator.runtime_state_for(STICKY_ID),
            Some(RuntimeState::Suspended)
        );
    }

    #[tokio::test]
    async fn permission_loss_marks_unavailable_and_never_reconnects() {
        let harness = Harness::active(false);
        harness.install_observer().await;

        harness
            .coordinator
            .handle(STICKY_ID, BindingEvent::PermissionRevoked)
            .await
            .unwrap();

        let binding = harness
            .repository
            .binding(STICKY_ID)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(binding.state, BindingState::Unavailable);
        assert_eq!(binding.runtime_window_id, None);
        assert_eq!(
            harness.coordinator.runtime_state_for(STICKY_ID),
            Some(RuntimeState::Unavailable)
        );
    }

    #[tokio::test]
    async fn target_close_is_durable_even_when_runtime_cleanup_fails() {
        let harness = Harness::active(false);
        harness.install_observer().await;
        harness.observers.fail_stop.store(true, Ordering::SeqCst);
        harness.native.fail_suspend.store(true, Ordering::SeqCst);

        assert!(harness
            .coordinator
            .handle(STICKY_ID, BindingEvent::TargetClosed)
            .await
            .is_err());

        let binding = harness
            .repository
            .binding(STICKY_ID)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(binding.state, BindingState::TargetClosed);
        assert_eq!(binding.runtime_pid, None);
        assert_eq!(binding.runtime_window_id, None);
        assert!(harness
            .coordinator
            .inner
            .active_observers
            .lock()
            .unwrap()
            .contains_key(STICKY_ID));
    }

    #[tokio::test]
    async fn target_close_does_not_stop_observing_when_durable_transition_fails() {
        let harness = Harness::active(false);
        harness.install_observer().await;
        harness
            .repository
            .fail_set_state
            .store(true, Ordering::SeqCst);

        assert!(harness
            .coordinator
            .handle(STICKY_ID, BindingEvent::TargetClosed)
            .await
            .is_err());

        let binding = harness
            .repository
            .binding(STICKY_ID)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(binding.state, BindingState::Active);
        assert_eq!(binding.runtime_window_id, Some(i64::from(TARGET_WINDOW_ID)));
        assert!(harness
            .coordinator
            .inner
            .active_observers
            .lock()
            .unwrap()
            .contains_key(STICKY_ID));
        assert!(harness.native.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn permission_loss_is_durable_even_when_order_out_fails() {
        let harness = Harness::active(false);
        harness.native.fail_suspend.store(true, Ordering::SeqCst);

        assert!(harness
            .coordinator
            .handle(STICKY_ID, BindingEvent::PermissionRevoked)
            .await
            .is_err());

        let binding = harness
            .repository
            .binding(STICKY_ID)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(binding.state, BindingState::Unavailable);
        assert_eq!(binding.runtime_window_id, None);
    }

    #[tokio::test]
    async fn observer_teardown_precedes_rebind_unbind_and_delete_detach() {
        for operation in ["rebind", "unbind", "delete"] {
            let harness = Harness::active(false);
            harness.install_observer().await;

            match operation {
                "rebind" => {
                    harness
                        .coordinator
                        .bind(STICKY_ID, descriptor())
                        .await
                        .unwrap();
                }
                "unbind" => harness.coordinator.unbind(STICKY_ID).await.unwrap(),
                "delete" => harness.coordinator.prepare_delete(STICKY_ID).await.unwrap(),
                _ => unreachable!(),
            }

            let log = harness.observers.order_log.lock().unwrap().clone();
            assert_eq!(&log[..2], &["observer_stop", "detach"], "{operation}");
        }
    }

    #[tokio::test]
    async fn queued_event_from_replaced_observer_cannot_mutate_new_binding() {
        let harness = Harness::active(false);
        harness.install_observer().await;
        let old_generation = harness
            .coordinator
            .inner
            .active_observers
            .lock()
            .unwrap()
            .get(STICKY_ID)
            .unwrap()
            .generation;

        harness
            .coordinator
            .bind(STICKY_ID, descriptor())
            .await
            .unwrap();
        harness.native.calls.lock().unwrap().clear();

        harness
            .coordinator
            .handle_observed(STICKY_ID, old_generation, BindingEvent::TargetClosed)
            .await
            .unwrap();

        let binding = harness
            .repository
            .binding(STICKY_ID)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(binding.state, BindingState::Active);
        assert_eq!(binding.runtime_window_id, Some(i64::from(TARGET_WINDOW_ID)));
        assert!(harness.native.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn prepare_delete_leaves_binding_terminal_before_sticky_delete() {
        let harness = Harness::active(false);
        harness.install_observer().await;

        harness.coordinator.prepare_delete(STICKY_ID).await.unwrap();

        let binding = harness
            .repository
            .binding(STICKY_ID)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(binding.state, BindingState::Unavailable);
        assert_eq!(binding.runtime_pid, None);
        assert_eq!(binding.runtime_window_id, None);
        assert!(!harness
            .coordinator
            .inner
            .active_observers
            .lock()
            .unwrap()
            .contains_key(STICKY_ID));
        assert!(!harness.coordinator.is_attached(STICKY_ID));
    }

    #[tokio::test]
    async fn reopen_revalidates_exact_target_then_commits_manual_visibility() {
        let harness = Harness::active(true);
        harness
            .coordinator
            .set_runtime_state(STICKY_ID, RuntimeState::ManuallyHidden);

        harness.coordinator.reopen(STICKY_ID).await.unwrap();

        assert_eq!(
            *harness.catalog.identities.lock().unwrap(),
            vec![(TARGET_PID, TARGET_WINDOW_ID)]
        );
        assert_eq!(
            *harness.native.calls.lock().unwrap(),
            vec![
                NativeCall::Attach(TARGET_WINDOW_ID),
                NativeCall::Resume(TARGET_WINDOW_ID)
            ]
        );
        assert_eq!(*harness.windows.reopen_count.lock().unwrap(), 1);
        assert!(!harness.repository.hidden.load(Ordering::SeqCst));
        assert_eq!(
            harness.coordinator.runtime_state_for(STICKY_ID),
            Some(RuntimeState::Visible)
        );
    }

    #[tokio::test]
    async fn reopening_while_target_is_suspended_restores_position_before_clearing_hide() {
        let harness = Harness::active(true);
        harness
            .coordinator
            .set_runtime_state(STICKY_ID, RuntimeState::SuspendedManuallyHidden);

        harness.coordinator.reopen(STICKY_ID).await.unwrap();

        assert!(!harness.repository.hidden.load(Ordering::SeqCst));
        assert!(harness.catalog.identities.lock().unwrap().is_empty());
        assert!(harness.native.calls.lock().unwrap().is_empty());
        assert_eq!(*harness.windows.reopen_count.lock().unwrap(), 1);
        assert_eq!(
            harness.coordinator.runtime_state_for(STICKY_ID),
            Some(RuntimeState::Suspended)
        );

        harness
            .coordinator
            .handle(STICKY_ID, BindingEvent::TargetRestored)
            .await
            .unwrap();
        assert_eq!(
            *harness.native.calls.lock().unwrap(),
            vec![NativeCall::Resume(TARGET_WINDOW_ID)]
        );
        assert_eq!(*harness.windows.reopen_count.lock().unwrap(), 1);
        assert_eq!(
            harness.coordinator.runtime_state_for(STICKY_ID),
            Some(RuntimeState::Visible)
        );
    }

    #[tokio::test]
    async fn suspended_reopen_position_failure_keeps_manual_hide_and_order_out() {
        let harness = Harness::active(true);
        harness
            .coordinator
            .set_runtime_state(STICKY_ID, RuntimeState::SuspendedManuallyHidden);
        harness.windows.fail_reopen.store(true, Ordering::SeqCst);

        assert!(harness.coordinator.reopen(STICKY_ID).await.is_err());

        assert!(harness.repository.hidden.load(Ordering::SeqCst));
        assert_eq!(
            *harness.native.calls.lock().unwrap(),
            vec![NativeCall::Suspend]
        );
        assert_eq!(
            harness.coordinator.runtime_state_for(STICKY_ID),
            Some(RuntimeState::SuspendedManuallyHidden)
        );
    }

    #[tokio::test]
    async fn suspended_reopen_hidden_commit_failure_keeps_note_ordered_out() {
        let harness = Harness::active(true);
        harness
            .coordinator
            .set_runtime_state(STICKY_ID, RuntimeState::SuspendedManuallyHidden);
        harness
            .repository
            .fail_set_hidden
            .store(true, Ordering::SeqCst);

        assert!(harness.coordinator.reopen(STICKY_ID).await.is_err());

        assert_eq!(*harness.windows.reopen_count.lock().unwrap(), 1);
        assert!(harness.repository.hidden.load(Ordering::SeqCst));
        assert_eq!(
            *harness.native.calls.lock().unwrap(),
            vec![NativeCall::Suspend]
        );
        assert_eq!(
            harness.coordinator.runtime_state_for(STICKY_ID),
            Some(RuntimeState::SuspendedManuallyHidden)
        );
    }

    #[tokio::test]
    async fn reopen_keeps_manual_hide_when_exact_target_is_unavailable() {
        let catalog = Arc::new(FakeCatalog::with_results(vec![Ok(false)]));
        let harness = Harness::with_catalog(true, catalog);
        harness
            .coordinator
            .set_runtime_state(STICKY_ID, RuntimeState::ManuallyHidden);

        assert!(harness.coordinator.reopen(STICKY_ID).await.is_err());

        assert!(harness.repository.hidden.load(Ordering::SeqCst));
        assert!(harness.native.calls.lock().unwrap().is_empty());
        assert_eq!(*harness.windows.reopen_count.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn reopen_reveal_failure_keeps_hidden_and_suspends_runtime_window() {
        let harness = Harness::active(true);
        harness
            .coordinator
            .set_runtime_state(STICKY_ID, RuntimeState::ManuallyHidden);
        harness.windows.fail_reopen.store(true, Ordering::SeqCst);

        assert!(harness.coordinator.reopen(STICKY_ID).await.is_err());

        assert!(harness.repository.hidden.load(Ordering::SeqCst));
        assert_eq!(
            *harness.native.calls.lock().unwrap(),
            vec![
                NativeCall::Attach(TARGET_WINDOW_ID),
                NativeCall::Resume(TARGET_WINDOW_ID),
                NativeCall::Suspend
            ]
        );
        assert_eq!(
            harness.coordinator.runtime_state_for(STICKY_ID),
            Some(RuntimeState::ManuallyHidden)
        );
    }

    #[tokio::test]
    async fn reopen_attach_failure_keeps_hidden_and_transitions_terminally() {
        let harness = Harness::active(true);
        harness
            .coordinator
            .set_runtime_state(STICKY_ID, RuntimeState::ManuallyHidden);
        *harness.native.attach_results.lock().unwrap() =
            VecDeque::from([Err(AppError::Other("attach failed".into()))]);

        assert!(harness.coordinator.reopen(STICKY_ID).await.is_err());

        let binding = harness
            .repository
            .binding(STICKY_ID)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(binding.state, BindingState::Unavailable);
        assert_eq!(binding.runtime_pid, None);
        assert_eq!(binding.runtime_window_id, None);
        assert!(harness.repository.hidden.load(Ordering::SeqCst));
        assert_eq!(*harness.windows.reopen_count.lock().unwrap(), 0);
        assert_eq!(
            harness.coordinator.runtime_state_for(STICKY_ID),
            Some(RuntimeState::Unavailable)
        );
    }

    #[tokio::test]
    async fn reopen_hidden_commit_failure_suspends_runtime_window() {
        let harness = Harness::active(true);
        harness
            .coordinator
            .set_runtime_state(STICKY_ID, RuntimeState::ManuallyHidden);
        harness
            .repository
            .fail_set_hidden
            .store(true, Ordering::SeqCst);

        assert!(harness.coordinator.reopen(STICKY_ID).await.is_err());

        assert!(harness.repository.hidden.load(Ordering::SeqCst));
        assert_eq!(
            harness.native.calls.lock().unwrap().last(),
            Some(&NativeCall::Suspend)
        );
        assert_eq!(
            harness.coordinator.runtime_state_for(STICKY_ID),
            Some(RuntimeState::ManuallyHidden)
        );
    }

    #[tokio::test]
    async fn unbind_retries_pin_restoration_after_binding_was_removed() {
        let harness = Harness::active(false);
        harness.install_observer().await;
        harness
            .windows
            .fail_restore_pinning
            .store(true, Ordering::SeqCst);

        assert!(harness.coordinator.unbind(STICKY_ID).await.is_err());
        assert!(harness
            .repository
            .binding(STICKY_ID)
            .await
            .unwrap()
            .is_none());
        assert_eq!(*harness.windows.binding_removed_count.lock().unwrap(), 1);

        harness.coordinator.unbind(STICKY_ID).await.unwrap();
        assert_eq!(*harness.windows.restore_pin_count.lock().unwrap(), 2);
        assert_eq!(*harness.windows.binding_removed_count.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn reconnecting_a_manually_hidden_note_never_opens_it_visibly() {
        let harness = Harness::active(true);
        {
            let mut binding = harness.repository.binding.lock().unwrap();
            let binding = binding.as_mut().unwrap();
            binding.state = BindingState::ReconnectRequired;
            binding.runtime_pid = None;
            binding.runtime_window_id = None;
        }

        harness
            .coordinator
            .bind(STICKY_ID, descriptor())
            .await
            .unwrap();

        assert_eq!(
            *harness.windows.ensure_keep_hidden.lock().unwrap(),
            vec![true]
        );
        assert_eq!(
            *harness.native.calls.lock().unwrap(),
            vec![NativeCall::Attach(TARGET_WINDOW_ID), NativeCall::Suspend]
        );
        assert_eq!(
            harness.coordinator.runtime_state_for(STICKY_ID),
            Some(RuntimeState::ManuallyHidden)
        );
        assert!(harness.repository.hidden.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn rebind_detach_failure_cannot_leave_an_unobserved_active_binding() {
        let harness = Harness::active(false);
        harness.install_observer().await;
        harness.native.fail_detach.store(true, Ordering::SeqCst);

        assert!(harness
            .coordinator
            .bind(STICKY_ID, descriptor())
            .await
            .is_err());

        let binding = harness
            .repository
            .binding(STICKY_ID)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(binding.state, BindingState::Active);
        assert_eq!(binding.runtime_pid, Some(i64::from(TARGET_PID)));
        assert_eq!(binding.runtime_window_id, Some(i64::from(TARGET_WINDOW_ID)));
        assert!(harness
            .coordinator
            .inner
            .active_observers
            .lock()
            .unwrap()
            .contains_key(STICKY_ID));
    }

    #[tokio::test]
    async fn unbind_db_failure_cannot_leave_an_unobserved_active_binding() {
        let harness = Harness::active(false);
        harness.install_observer().await;
        harness.repository.fail_unbind.store(true, Ordering::SeqCst);

        assert!(harness.coordinator.unbind(STICKY_ID).await.is_err());

        let binding = harness
            .repository
            .binding(STICKY_ID)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(binding.state, BindingState::Active);
        assert_eq!(binding.runtime_pid, Some(i64::from(TARGET_PID)));
        assert_eq!(binding.runtime_window_id, Some(i64::from(TARGET_WINDOW_ID)));
        assert!(harness
            .coordinator
            .inner
            .active_observers
            .lock()
            .unwrap()
            .contains_key(STICKY_ID));
    }

    fn descriptor() -> WindowDescriptor {
        WindowDescriptor {
            app_bundle_id: "com.example.Target".into(),
            app_name: "Target".into(),
            app_icon_data_url: None,
            pid: TARGET_PID,
            window_id: TARGET_WINDOW_ID,
            window_title: "Document".into(),
        }
    }

    fn test_binding() -> WindowBinding {
        WindowBinding {
            sticky_id: STICKY_ID.into(),
            app_bundle_id: "com.example.Target".into(),
            app_name: "Target".into(),
            window_title: "Document".into(),
            runtime_pid: Some(i64::from(TARGET_PID)),
            runtime_window_id: Some(i64::from(TARGET_WINDOW_ID)),
            state: BindingState::Active,
            prior_pinned: 0,
            created_at: 0,
            updated_at: 0,
        }
    }
}
