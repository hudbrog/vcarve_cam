#[cfg(feature = "bundled-ui")]
include!(concat!(env!("OUT_DIR"), "/bundled_assets.rs"));

pub fn assets() -> Option<cam_server::Assets> {
    #[cfg(feature = "bundled-ui")]
    {
        Some(cam_server::embedded_assets(BUNDLED_ASSETS))
    }
    #[cfg(not(feature = "bundled-ui"))]
    {
        None
    }
}
