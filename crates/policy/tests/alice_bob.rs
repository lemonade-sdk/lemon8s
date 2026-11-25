use policy::{Compartment, Denial, Label, Policy, UserId};

struct System {
    policy: Policy,
    alice: UserId,
    bob: UserId,
    mallory: UserId,
    finance: Label,
    legal: Label,
    finance_legal: Label,
}

impl System {
    fn new() -> Self {
        let finance = Label::new([Compartment::new("finance")]);
        let legal = Label::new([Compartment::new("legal")]);
        let finance_legal = Label::new([Compartment::new("finance"), Compartment::new("legal")]);
        let alice = UserId::new("alice");
        let bob = UserId::new("bob");
        let mallory = UserId::new("mallory");
        let mut policy = Policy::new();
        policy.add_user(alice.clone(), finance_legal.clone());
        policy.add_user(bob.clone(), finance.clone());
        policy.add_user(mallory.clone(), Label::public());
        Self {
            policy,
            alice,
            bob,
            mallory,
            finance,
            legal,
            finance_legal,
        }
    }
}

#[test]
fn session_creation_is_bounded_by_clearance() {
    let mut sys = System::new();

    assert!(
        sys.policy
            .create_session(&sys.alice, sys.finance.clone())
            .is_ok()
    );
    assert!(
        sys.policy
            .create_session(&sys.alice, sys.legal.clone())
            .is_ok()
    );
    assert!(
        sys.policy
            .create_session(&sys.alice, sys.finance_legal.clone())
            .is_ok()
    );

    assert_eq!(
        sys.policy.create_session(&sys.bob, sys.legal.clone()),
        Err(Denial::LabelExceedsClearance {
            requested: sys.legal.clone(),
            clearance: sys.finance.clone(),
        })
    );

    assert!(
        sys.policy
            .create_session(&sys.mallory, Label::public())
            .is_ok()
    );
    assert_eq!(
        sys.policy.create_session(&sys.mallory, sys.finance.clone()),
        Err(Denial::LabelExceedsClearance {
            requested: sys.finance.clone(),
            clearance: Label::public(),
        })
    );
}

#[test]
fn only_the_owner_can_open_a_session() {
    let mut sys = System::new();
    let mixed = sys
        .policy
        .create_session(&sys.alice, sys.finance_legal.clone())
        .unwrap();
    let finance_only = sys
        .policy
        .create_session(&sys.alice, sys.finance.clone())
        .unwrap();

    assert_eq!(
        sys.policy.open_session(&sys.bob, &mixed),
        Err(Denial::NotSessionOwner)
    );
    assert_eq!(
        sys.policy.open_session(&sys.bob, &finance_only),
        Err(Denial::NotSessionOwner)
    );
    assert!(sys.policy.open_session(&sys.alice, &mixed).is_ok());
    assert!(sys.policy.open_session(&sys.alice, &finance_only).is_ok());
}

#[test]
fn a_revoked_user_is_denied_everywhere() {
    let mut sys = System::new();
    let session = sys
        .policy
        .create_session(&sys.alice, sys.finance.clone())
        .unwrap();

    sys.policy.revoke_user(&sys.alice).unwrap();

    assert_eq!(
        sys.policy.create_session(&sys.alice, sys.finance.clone()),
        Err(Denial::UserRevoked)
    );
    assert_eq!(
        sys.policy.open_session(&sys.alice, &session),
        Err(Denial::UserRevoked)
    );
    assert_eq!(
        sys.policy.open_request(&sys.alice, &session, None),
        Err(Denial::UserRevoked)
    );
}

#[test]
fn clearance_reduction_blocks_resuming_a_session_above_it() {
    let mut sys = System::new();
    let mixed = sys
        .policy
        .create_session(&sys.alice, sys.finance_legal.clone())
        .unwrap();
    assert!(sys.policy.open_session(&sys.alice, &mixed).is_ok());

    sys.policy
        .set_clearance(&sys.alice, sys.finance.clone())
        .unwrap();
    assert_eq!(
        sys.policy.open_session(&sys.alice, &mixed),
        Err(Denial::LabelExceedsClearance {
            requested: sys.finance_legal.clone(),
            clearance: sys.finance.clone(),
        })
    );
}
