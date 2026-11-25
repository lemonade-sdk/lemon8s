use crate::ids::UserId;
use crate::label::Label;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserStatus {
    Active,
    Revoked,
}

#[derive(Debug, Clone)]
pub struct User {
    pub id: UserId,
    pub clearance: Label,
    pub status: UserStatus,
}
