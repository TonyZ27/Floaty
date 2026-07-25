use super::{BindingEvent, RuntimeState};

pub fn reduce(state: RuntimeState, event: BindingEvent) -> RuntimeState {
    match event {
        BindingEvent::UserHidNote => match state {
            RuntimeState::Visible | RuntimeState::ManuallyHidden => RuntimeState::ManuallyHidden,
            RuntimeState::Suspended | RuntimeState::SuspendedManuallyHidden => {
                RuntimeState::SuspendedManuallyHidden
            }
            unavailable => unavailable,
        },
        BindingEvent::UserOpenedNote => match state {
            RuntimeState::ManuallyHidden => RuntimeState::Visible,
            RuntimeState::SuspendedManuallyHidden => RuntimeState::Suspended,
            current => current,
        },
        BindingEvent::TargetMinimized => match state {
            RuntimeState::Visible => RuntimeState::Suspended,
            RuntimeState::ManuallyHidden => RuntimeState::SuspendedManuallyHidden,
            current => current,
        },
        BindingEvent::TargetRestored => match state {
            RuntimeState::Suspended => RuntimeState::Visible,
            RuntimeState::SuspendedManuallyHidden => RuntimeState::ManuallyHidden,
            current => current,
        },
        BindingEvent::TargetClosed => match state {
            RuntimeState::Visible
            | RuntimeState::Suspended
            | RuntimeState::ManuallyHidden
            | RuntimeState::SuspendedManuallyHidden => RuntimeState::TargetClosed,
            unavailable => unavailable,
        },
        BindingEvent::SimilarWindowOpened => state,
        BindingEvent::PermissionRevoked => RuntimeState::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assignment::{BindingEvent, BindingState, RuntimeState};

    #[test]
    fn states_serialize_with_stable_snake_case_names() {
        assert_eq!(
            serde_json::to_string(&BindingState::ReconnectRequired).unwrap(),
            "\"reconnect_required\""
        );
        assert_eq!(
            serde_json::to_string(&RuntimeState::ManuallyHidden).unwrap(),
            "\"manually_hidden\""
        );
        assert_eq!(
            serde_json::to_string(&RuntimeState::SuspendedManuallyHidden).unwrap(),
            "\"suspended_manually_hidden\""
        );
    }

    #[test]
    fn manual_hide_wins_over_target_restore() {
        let state = reduce(RuntimeState::Visible, BindingEvent::UserHidNote);
        assert_eq!(state, RuntimeState::ManuallyHidden);
        assert_eq!(
            reduce(state, BindingEvent::TargetRestored),
            RuntimeState::ManuallyHidden
        );
    }

    #[test]
    fn target_minimize_is_runtime_only() {
        assert_eq!(
            reduce(RuntimeState::Visible, BindingEvent::TargetMinimized),
            RuntimeState::Suspended
        );
        assert_eq!(
            reduce(RuntimeState::Suspended, BindingEvent::TargetRestored),
            RuntimeState::Visible
        );
    }

    #[test]
    fn opening_after_minimize_then_hide_remains_suspended() {
        let state = reduce(RuntimeState::Visible, BindingEvent::TargetMinimized);
        let state = reduce(state, BindingEvent::UserHidNote);
        assert_eq!(state, RuntimeState::SuspendedManuallyHidden);

        let state = reduce(state, BindingEvent::UserOpenedNote);
        assert_eq!(state, RuntimeState::Suspended);
        assert_eq!(
            reduce(state, BindingEvent::TargetRestored),
            RuntimeState::Visible
        );
    }

    #[test]
    fn restore_after_hide_then_minimize_remains_manually_hidden() {
        let state = reduce(RuntimeState::Visible, BindingEvent::UserHidNote);
        let state = reduce(state, BindingEvent::TargetMinimized);
        assert_eq!(state, RuntimeState::SuspendedManuallyHidden);

        let state = reduce(state, BindingEvent::TargetRestored);
        assert_eq!(state, RuntimeState::ManuallyHidden);
        assert_eq!(
            reduce(state, BindingEvent::UserOpenedNote),
            RuntimeState::Visible
        );
    }

    #[test]
    fn target_close_never_auto_reconnects() {
        assert_eq!(
            reduce(RuntimeState::Visible, BindingEvent::TargetClosed),
            RuntimeState::TargetClosed
        );
        assert_eq!(
            reduce(
                RuntimeState::TargetClosed,
                BindingEvent::SimilarWindowOpened
            ),
            RuntimeState::TargetClosed
        );
    }

    #[test]
    fn user_open_does_not_bypass_rebind_required_states() {
        for state in [
            RuntimeState::TargetClosed,
            RuntimeState::ReconnectRequired,
            RuntimeState::Unavailable,
        ] {
            assert_eq!(reduce(state, BindingEvent::UserOpenedNote), state);
        }
    }

    #[test]
    fn permission_revocation_makes_every_state_unavailable() {
        for state in all_states() {
            assert_eq!(
                reduce(state, BindingEvent::PermissionRevoked),
                RuntimeState::Unavailable
            );
        }
    }

    #[test]
    fn every_event_has_a_transition_from_every_state() {
        let visible = RuntimeState::Visible;
        let suspended = RuntimeState::Suspended;
        let manually_hidden = RuntimeState::ManuallyHidden;
        let suspended_manually_hidden = RuntimeState::SuspendedManuallyHidden;
        let target_closed = RuntimeState::TargetClosed;
        let reconnect_required = RuntimeState::ReconnectRequired;
        let unavailable = RuntimeState::Unavailable;
        let expected = [
            [
                manually_hidden,
                visible,
                suspended,
                visible,
                target_closed,
                visible,
                unavailable,
            ],
            [
                suspended_manually_hidden,
                suspended,
                suspended,
                visible,
                target_closed,
                suspended,
                unavailable,
            ],
            [
                manually_hidden,
                visible,
                suspended_manually_hidden,
                manually_hidden,
                target_closed,
                manually_hidden,
                unavailable,
            ],
            [
                suspended_manually_hidden,
                suspended,
                suspended_manually_hidden,
                manually_hidden,
                target_closed,
                suspended_manually_hidden,
                unavailable,
            ],
            [
                target_closed,
                target_closed,
                target_closed,
                target_closed,
                target_closed,
                target_closed,
                unavailable,
            ],
            [
                reconnect_required,
                reconnect_required,
                reconnect_required,
                reconnect_required,
                reconnect_required,
                reconnect_required,
                unavailable,
            ],
            [
                unavailable,
                unavailable,
                unavailable,
                unavailable,
                unavailable,
                unavailable,
                unavailable,
            ],
        ];

        for (state_index, state) in all_states().into_iter().enumerate() {
            for (event_index, event) in all_events().into_iter().enumerate() {
                assert_eq!(
                    reduce(state, event),
                    expected[state_index][event_index],
                    "unexpected transition from {state:?} on {event:?}"
                );
            }
        }
    }

    fn all_states() -> [RuntimeState; 7] {
        [
            RuntimeState::Visible,
            RuntimeState::Suspended,
            RuntimeState::ManuallyHidden,
            RuntimeState::SuspendedManuallyHidden,
            RuntimeState::TargetClosed,
            RuntimeState::ReconnectRequired,
            RuntimeState::Unavailable,
        ]
    }

    fn all_events() -> [BindingEvent; 7] {
        [
            BindingEvent::UserHidNote,
            BindingEvent::UserOpenedNote,
            BindingEvent::TargetMinimized,
            BindingEvent::TargetRestored,
            BindingEvent::TargetClosed,
            BindingEvent::SimilarWindowOpened,
            BindingEvent::PermissionRevoked,
        ]
    }
}
