//! Combine a job token with a shorter-lived per-call token.
pub(crate) struct AnimationStops<'a>(pub &'a dyn enough::Stop, pub &'a dyn enough::Stop);

impl enough::Stop for AnimationStops<'_> {
    fn check(&self) -> Result<(), enough::StopReason> {
        self.0.check()?;
        self.1.check()
    }

    fn may_stop(&self) -> bool {
        self.0.may_stop() || self.1.may_stop()
    }
}
