//! Enable the application's bundled translations without replacing translation text.
use anyhow::{bail, Result};

pub fn patch_source(source: &str) -> Result<String> {
    const PAIRS: [(&str, &str); 2] = [
        ("c=s?.get(`enable_i18n`,!1),t[0]=s,t[1]=c", "c=!0/*jokerdeck-bundled-i18n-v1*/,t[0]=s,t[1]=c"),
        ("o=a?.get(`enable_i18n`,!1),t[0]=a,t[1]=o", "o=!0/*jokerdeck-bundled-i18n-v1*/,t[0]=a,t[1]=o"),
    ];
    if source.contains("/*jokerdeck-bundled-i18n-v1*/") { return Ok(source.to_owned()); }
    let matches: Vec<_> = PAIRS.iter().filter(|(anchor, _)| source.contains(anchor)).collect();
    if matches.is_empty() {
        // Newer bundles can expose translations through a different gate. Keep
        // the official locale override usable without mutating an unknown path.
        return Ok(source.to_owned());
    }
    if matches.len() != 1 || source.matches(matches[0].0).count() != 1 {
        bail!("Codex 中文语言包入口不唯一，未修改应用副本");
    }
    Ok(source.replacen(matches[0].0, matches[0].1, 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keeps_locale_selection_and_is_idempotent() {
        let input = "c=s?.get(`enable_i18n`,!1),t[0]=s,t[1]=c;localeOverride=chosen";
        let patched = patch_source(input).unwrap();
        assert!(patched.contains("localeOverride=chosen"));
        assert_eq!(patch_source(&patched).unwrap(), patched);
        assert_eq!(patch_source("unsupported bundle").unwrap(), "unsupported bundle");
        assert!(patch_source(&(input.to_owned()+input)).is_err());
    }
}
