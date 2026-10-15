//! One startup branch, no fallback and no tee. Runtime legacy remains default.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) enum Selection {
    #[default]
    Legacy,
    #[cfg(feature = "offline-puresignal-owner-adapters")]
    SharedReview,
    #[cfg(feature = "managed-puresignal")]
    Shared,
}
pub(crate) fn select<T, E>(
    selection: Selection,
    legacy: impl FnOnce() -> Result<T, E>,
    #[cfg(feature = "offline-puresignal-owner-adapters")] shared: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    match selection {
        Selection::Legacy => legacy(),
        #[cfg(feature = "offline-puresignal-owner-adapters")]
        Selection::SharedReview => shared(),
        #[cfg(feature = "managed-puresignal")]
        Selection::Shared => shared(),
    }
}
impl Selection {
    pub(crate) fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.unwrap_or("legacy").trim() {
            "legacy" => Ok(Self::Legacy),
            #[cfg(feature = "managed-puresignal")]
            "shared" => Ok(Self::Shared),
            #[cfg(not(feature = "managed-puresignal"))]
            "shared" => Err("shared TX requires the managed-puresignal build feature".into()),
            _ => Err("SATURN_BRIDGE_TX_OWNER must be legacy or shared".into()),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_selects_only_legacy() {
        assert_eq!(Selection::parse(None).unwrap(), Selection::Legacy);
        assert_eq!(Selection::parse(Some("legacy")).unwrap(), Selection::Legacy);
        assert!(Selection::parse(Some("unknown")).is_err());
        assert_eq!(Selection::default(), Selection::Legacy);
        let value = select(
            Selection::default(),
            || Ok::<_, ()>(3),
            #[cfg(feature = "offline-puresignal-owner-adapters")]
            || panic!("parallel shared owner"),
        );
        assert_eq!(value, Ok(3));
    }
    #[cfg(feature = "managed-puresignal")]
    #[test]
    fn explicit_shared_startup_failure_never_selects_legacy() {
        assert_eq!(Selection::parse(Some("shared")).unwrap(), Selection::Shared);
        assert_eq!(
            select(
                Selection::Shared,
                || panic!("legacy fallback"),
                || Err::<(), _>("failed acquisition")
            ),
            Err("failed acquisition")
        );
    }
    #[cfg(feature = "offline-puresignal-owner-adapters")]
    #[test]
    fn shared_failure_never_falls_back_to_legacy() {
        assert_eq!(
            select(
                Selection::SharedReview,
                || panic!("legacy fallback"),
                || Err::<(), _>("qualification required")
            ),
            Err("qualification required")
        );
    }
}
