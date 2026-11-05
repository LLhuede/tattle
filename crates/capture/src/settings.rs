//! Environment settings. Each is `TATTLE_<NAME>`; the old `CV_<NAME>`
//! (before the tool was renamed from cv to tattle, 2026-11-04) still works,
//! so existing setups keep running.

/// The value of `TATTLE_<name>`, else `CV_<name>`. Empty values count as unset.
pub fn var(name: &str) -> Option<String> {
    [format!("TATTLE_{name}"), format!("CV_{name}")]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
}

/// As [`var`], for paths.
pub fn var_os(name: &str) -> Option<std::ffi::OsString> {
    [format!("TATTLE_{name}"), format!("CV_{name}")]
        .iter()
        .find_map(|k| std::env::var_os(k).filter(|v| !v.is_empty()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_new_name_wins_and_the_old_one_still_works() {
        // Unique names so parallel tests don't collide.
        unsafe {
            std::env::set_var("CV_ZZ_PROBE_A", "old");
            std::env::set_var("TATTLE_ZZ_PROBE_B", "new");
            std::env::set_var("CV_ZZ_PROBE_B", "old");
            std::env::set_var("TATTLE_ZZ_PROBE_C", "");
        }
        assert_eq!(super::var("ZZ_PROBE_A").as_deref(), Some("old"));
        assert_eq!(super::var("ZZ_PROBE_B").as_deref(), Some("new"));
        assert_eq!(super::var("ZZ_PROBE_C"), None);
    }
}
