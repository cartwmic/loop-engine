#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    Active,
    Final,
    Terminated,
}

impl Lifecycle {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Final | Self::Terminated)
    }
}
