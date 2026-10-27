//! The system region new documents follow (#838): the paper File › New starts on, the default
//! measurement unit and the decimal separator.
//!
//! - Windows: the preferred language with the home location from Settings › Time & language ›
//!   Region (`GlobalizationPreferences.HomeGeographicRegion`, a safe Windows Runtime API), since
//!   the language list alone says `en-US` for an English interface in Poland.
//! - Elsewhere: the locale the UI crate reads ([`localoffice_ui_egui::i18n::system_region`]).

/// The system region.
pub fn system() -> localoffice_engine::Region {
    // Runs without preferences (agents' and screenshot runs) behave the same on every machine:
    // en-US unless `LOCALOFFICE_REGION` says otherwise.
    if std::env::var_os("LOCALOFFICE_NO_PREFS").is_some() && std::env::var_os("LOCALOFFICE_REGION").is_none() {
        return localoffice_engine::Region::default();
    }
    let region = localoffice_ui_egui::i18n::system_region();
    #[cfg(target_os = "windows")]
    {
        if std::env::var_os("LOCALOFFICE_REGION").is_none() {
            return match windows::System::UserProfile::GlobalizationPreferences::HomeGeographicRegion() {
                Ok(home) => region.with_territory(&home.to_string()),
                Err(e) => {
                    log::debug!("system home region: {e}");
                    region
                }
            };
        }
    }
    region
}
