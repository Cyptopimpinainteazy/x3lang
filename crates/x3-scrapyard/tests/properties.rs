use proptest::prelude::*;
use x3_scrapyard::LifecycleState;

fn state() -> impl Strategy<Value = LifecycleState> {
    prop_oneof![
        Just(LifecycleState::Active),
        Just(LifecycleState::Warning),
        Just(LifecycleState::Probation),
        Just(LifecycleState::Quarantined),
        Just(LifecycleState::Suspended),
        Just(LifecycleState::Terminated),
        Just(LifecycleState::Archived),
    ]
}

proptest! {
    #[test]
    fn terminal_states_never_return_to_active(current in state()) {
        if current.is_terminal() {
            prop_assert!(!current.can_transition_to(LifecycleState::Active));
            prop_assert!(current.denies_execution());
        }
    }

    #[test]
    fn transition_relation_is_directional(left in state(), right in state()) {
        if left.can_transition_to(right) {
            prop_assert_ne!(left, LifecycleState::Archived);
        }
    }
}
