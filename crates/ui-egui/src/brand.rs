//! The PhotoCraft brand mark at the left of the title bar: the app icon (the kitsune,
//! `assets/app-icon/`, see its README), where Photoshop shows its "Ps" tile. The PNG carries the
//! icon's rounded corners; it is decoded once per context into a mipmapped texture, so it stays
//! crisp at the title bar's size on any display scale.

use egui::{Color32, Context, Id, Rect, TextureHandle, TextureOptions, Ui, pos2};
use photocraft_codecs::{ChannelLayout, SampleType};

/// 128 px: sharp at 20 pt on a 3× display, and the mipmaps keep it clean at 1×.
#[cfg_attr(feature = "stucent-brand", allow(dead_code))]
const ICON_PNG: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/128x128/apps/ai.storyteller.photocraft.png");

/// The icon's pixels (one transparent pixel if it couldn't be decoded, which a test rules out).
#[cfg_attr(feature = "stucent-brand", allow(dead_code))]
fn decode() -> egui::ColorImage {
    let rgba = photocraft_codecs::decode(ICON_PNG).ok().map(|img| img.convert(ChannelLayout::Rgba, SampleType::U8));
    match rgba {
        Some(img) if img.data().len() == img.width() as usize * img.height() as usize * 4 => {
            egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.data())
        }
        _ => egui::ColorImage::filled([1, 1], Color32::TRANSPARENT),
    }
}

/// The icon texture, uploaded on first use and kept in the context.
#[cfg_attr(feature = "stucent-brand", allow(dead_code))]
fn texture(ctx: &Context) -> TextureHandle {
    let id = Id::new("photocraft-brand-mark");
    if let Some(tex) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return tex;
    }
    let options = TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..TextureOptions::LINEAR };
    let tex = ctx.load_texture("photocraft-brand-mark", decode(), options);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// Where the mark was painted this frame (tests and screenshots).
fn rect_id() -> Id {
    Id::new("photocraft-brand-mark-rect")
}

/// Paint the mark into `r` (square).
pub fn paint_mark(ui: &Ui, r: Rect) {
    // StuCent Photo Editor: a neutral tile (the accent colour and the image icon), not the
    // PhotoCraft kitsune.
    #[cfg(feature = "stucent-brand")]
    {
        let t = crate::theme::Tokens::get(ui.ctx());
        ui.painter().rect_filled(r, r.width() * 0.22, t.accent);
        crate::icons::paint(ui, r.shrink(r.width() * 0.18), "image", r.width() * 0.64, Color32::WHITE);
        ui.ctx().data_mut(|d| d.insert_temp(rect_id(), r));
        return;
    }
    #[allow(unreachable_code)]
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter().image(texture(ui.ctx()).id(), r, uv, Color32::WHITE);
    ui.ctx().data_mut(|d| d.insert_temp(rect_id(), r));
}

/// The rect the mark was last painted into.
#[cfg(test)]
pub fn mark_rect(ctx: &Context) -> Option<Rect> {
    ctx.data(|d| d.get_temp(rect_id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_decodes_to_a_rounded_128_px_tile() {
        let img = decode();
        assert_eq!(img.size, [128, 128]);
        // Rounded corners: the corner pixel is transparent, the centre opaque.
        assert_eq!(img.pixels[0].a(), 0);
        assert_eq!(img.pixels[64 * 128 + 64].a(), 255);
    }

    // The StuCent build paints a neutral tile instead (no texture).
    #[cfg(not(feature = "stucent-brand"))]
    #[test]
    fn the_title_bar_paints_the_icon_from_one_cached_texture() {
        let ctx = Context::default();
        crate::PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        // The system title bar hides the in-app mark (the OS draws its own icon), so paint the
        // app-drawn bar here.
        app.custom_titlebar = true;
        let mut frame = || {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::panels::title_bar(&mut app, ui));
            let uploads = out.textures_delta.set.values().flat_map(|d| d.iter()).filter(|d| d.image.size() == [128, 128]).count();
            out.textures_delta.clear();
            uploads
        };
        assert_eq!(frame(), 1, "uploaded on first use");
        assert_eq!(frame(), 0, "then reused");
        assert!(mark_rect(&ctx).is_some());
    }
}

/// The app's name as shown in the UI (title bar, window title, About, the start screen).
pub const APP_NAME: &str = if cfg!(feature = "stucent-brand") { "StuCent Photo Editor" } else { "PhotoCraft" };

/// The upstream product name that `stucent-brand` replaces in shown text.
pub const UPSTREAM_NAME: &str = "PhotoCraft";

/// Shown text with the upstream name replaced by [`APP_NAME`] (unchanged without `stucent-brand`).
pub fn rebrand(text: &str) -> String {
    if cfg!(feature = "stucent-brand") { text.replace(UPSTREAM_NAME, APP_NAME) } else { text.to_string() }
}

/// The StuCent build's tests: run with `cargo test -p photocraft-ui-egui --features stucent-brand`.
#[cfg(all(test, feature = "stucent-brand"))]
mod stucent_tests {
    use super::*;
    use crate::i18n::{self, Lang};

    #[test]
    fn rebrand_replaces_the_upstream_name() {
        assert_eq!(rebrand("About PhotoCraft"), "About StuCent Photo Editor");
        assert_eq!(rebrand("PhotoCraft について"), "StuCent Photo Editor について");
        assert_eq!(rebrand("Layer"), "Layer");
    }

    #[test]
    fn english_and_translated_ui_text_names_the_stucent_editor() {
        assert_eq!(i18n::tr(Lang::EN, "About PhotoCraft"), "About StuCent Photo Editor");
        assert_eq!(i18n::tr_id(Lang::EN, "help.about", "About PhotoCraft"), "About StuCent Photo Editor");
        for lang in Lang::all() {
            let about = i18n::tr_id(lang, "help.about", "About PhotoCraft");
            assert!(!about.contains(UPSTREAM_NAME), "{}: {about}", lang.code());
            assert!(about.contains(APP_NAME), "{}: {about}", lang.code());
        }
    }

    #[test]
    fn no_catalog_value_shows_the_upstream_name() {
        for lang in Lang::all() {
            for (ctx, src, _) in i18n::catalog_entries(lang) {
                let shown = match ctx.as_str() {
                    "" => i18n::tr(lang, &src).to_string(),
                    "@id" => i18n::tr_id(lang, &src, "").to_string(),
                    "@plural" => continue,
                    c => i18n::tr_ctx(lang, c, &src).to_string(),
                };
                assert!(!shown.contains(UPSTREAM_NAME), "{}: {src} → {shown}", lang.code());
            }
        }
    }

    #[test]
    fn community_links_are_gone() {
        for id in ["help.discord", "help.website", "help.artcraftWebsite", "help.github", "help.reportIssue"] {
            assert_eq!(crate::links::url_for(id), None, "{id}");
        }
        let app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let help: Vec<String> = crate::menus::menu_items(&app).into_iter().filter(|i| i.path.first().is_some_and(|p| p == "Help")).map(|i| i.id).collect();
        for id in ["help.discord", "help.website", "help.artcraftWebsite", "help.github", "help.reportIssue"] {
            assert!(!help.iter().any(|h| h == id), "{id} in {help:?}");
        }
        assert!(help.iter().any(|i| i == "help.about"), "{help:?}");
    }

    #[test]
    fn titles_name_the_stucent_editor() {
        let app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        assert_eq!(crate::panels::window_title(&app), "StuCent Photo Editor");
    }
}
