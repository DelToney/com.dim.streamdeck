use async_trait::async_trait;
use futures_util::future::join3;
use serde::{Deserialize, Serialize};
use serde_json::json;
use skia_safe::color_filters::matrix;
use skia_safe::image_filters::{blur, image};
use skia_safe::{BlendMode, Color, Image, Point, Rect};
use std::time::{Duration, Instant};
use stream_deck_sdk::action::Action;
use stream_deck_sdk::events::events::{
    AppearEvent, DidReceiveGlobalSettingsEvent, DidReceiveSettingsEvent, KeyEvent,
    SendToPluginEvent,
};
use stream_deck_sdk::get_settings;
use stream_deck_sdk::stream_deck::StreamDeck;
use tokio::time::sleep;

use crate::actions::search::SearchSettings;
use crate::canvas::enhancement::{blur_image, scale_image};
use crate::dim::events_sent::Selection;
use crate::dim::with_action;
use crate::json_string;
use crate::shared::{
    has_equipped_items, EQUIPPED_MARK, EXOTIC, GRAYSCALE, LEGENDARY, SHARED, SYNC, SYNC_DONE,
};
use crate::util::{
    bungify, bytes_to_skia_image, download_or_cache, prepare_render_empty, skia_image_to_b64,
};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PullItemSettings {
    pub(crate) item: Option<String>,
    pub(crate) label: Option<String>,
    pub(crate) subtitle: Option<String>,
    pub(crate) icon: Option<String>,
    pub(crate) overlay: Option<String>,
    pub(crate) element: Option<String>,
    #[serde(rename = "altAction")]
    pub(crate) alt_action: Option<String>,
    // "hold" | "double"
    #[serde(rename = "altActionTrigger")]
    pub(crate) alt_action_trigger: Option<String>,
    // "equip" for future use
    pub(crate) inventory: Option<bool>,
    #[serde(rename = "isExotic")]
    pub(crate) is_exotic: Option<bool>,
}

#[derive(Serialize)]
pub struct SendPullItem {
    pub(crate) item: String,
    pub(crate) context: String,
    pub(crate) equip: bool,
}

#[derive(Deserialize, Debug)]
struct PartialPluginSettings {
    pub(crate) grayscale: Option<bool>,
}

pub struct PullItemAction;

async fn render_action(settings: PullItemSettings, grayscale_enabled: bool) -> Option<Image> {
    if settings.item.is_none() || settings.icon.is_none() {
        return None;
    }

    let image = download_or_cache(bungify(settings.icon));
    let overlay = download_or_cache(bungify(settings.overlay));
    let element = download_or_cache(bungify(settings.element));

    let (image, overlay, element) = join3(image, overlay, element).await;

    if image.is_none() {
        return None;
    }

    let equipped = settings.inventory.unwrap_or_else(|| false)
        || has_equipped_items(settings.item.unwrap().clone()).await;

    let glass = bytes_to_skia_image(match settings.is_exotic {
        Some(true) => EXOTIC.to_vec(),
        _ => LEGENDARY.to_vec(),
    });

    let size = 144.0;
    let (mut surface, mut paint, _) = prepare_render_empty(size as i32);

    if !equipped && grayscale_enabled {
        paint.set_color_filter(Some(matrix(&GRAYSCALE)));
    }

    let image = bytes_to_skia_image(image.unwrap());
    let image = scale_image(image, size, size);

    let shift = 5.0;

    surface
        .canvas()
        .clear(Color::from_argb(0, 0, 0, 0))
        .draw_image_rect(
            image,
            None,
            Rect::new(shift, shift, size - shift, size - shift),
            &paint,
        );

    if overlay.is_some() {
        let overlay = scale_image(bytes_to_skia_image(overlay.unwrap()), size, size);
        surface.canvas().draw_image_rect(
            overlay,
            None,
            Rect::new(shift, shift, size - shift, size - shift),
            &paint,
        );
    }

    if equipped && !grayscale_enabled {
        let mark = bytes_to_skia_image(EQUIPPED_MARK.to_vec());
        surface.canvas().draw_image_rect(
            mark,
            None,
            Rect::new(10.0, size - 10.0 - 21.0, 10.0 + 21.0, size - 10.0),
            &paint,
        );
    }

    surface.canvas().draw_image_rect(
        glass,
        None,
        Rect::new(shift, shift, size - shift, size - shift),
        &paint,
    );

    if let Some(element) = element {
        let radius = 12.0;
        let margin = size - radius * 1.6;
        let element_margin = margin - 8.0;
        let element = bytes_to_skia_image(element);

        let filter = blur((8.0, 8.0), None, None, None);

        paint
            .set_color_filter(None)
            .set_color(Color::from_argb(225, 0, 0, 0))
            .set_image_filter(filter);

        surface
            .canvas()
            .draw_circle(Point::new(margin, margin), radius * 1.1, &paint);

        paint.set_alpha(255).set_image_filter(None);

        surface.canvas().draw_image_rect(
            element,
            None,
            Rect::new(
                element_margin,
                element_margin - 0.5,
                element_margin + 17.0,
                element_margin + 16.5,
            ),
            &paint,
        );
    }

    return Some(surface.image_snapshot());
}

pub fn loading_image(image: Image, degree: f32, done: bool) -> Option<Image> {
    let size = image.width() as f32;
    let (mut surface, mut paint, _) = prepare_render_empty(size as i32);
    let loading = blur_image(image, 6.0);

    surface
        .canvas()
        .draw_image_rect(loading, None, Rect::new(0.0, 0.0, size, size), &paint);

    paint
        .set_color(Color::from_argb(120, 0, 0, 0))
        .set_blend_mode(BlendMode::Multiply);

    surface
        .canvas()
        .draw_rect(Rect::new(0.0, 0.0, size, size), &paint);

    paint
        .set_color(Color::from_argb(255, 255, 255, 255))
        .set_blend_mode(BlendMode::SrcOver);

    let overlay = bytes_to_skia_image(if done {
        SYNC_DONE.to_vec()
    } else {
        SYNC.to_vec()
    });

    if done {
        surface
            .canvas()
            .draw_image_rect(overlay, None, Rect::new(0.0, 0.0, size, size), &paint);
    } else {
        surface
            .canvas()
            .rotate(degree, Some(Point::new(size / 2.0, size / 2.0)))
            .draw_image_rect(overlay, None, Rect::new(0.0, 0.0, size, size), &paint);
    }

    return Some(surface.image_snapshot());
}

impl PullItemAction {
    async fn update(
        &self,
        context: String,
        settings: PullItemSettings,
        loading: bool,
        sd: StreamDeck,
    ) {
        let global_settings = sd.global_settings().await.unwrap_or(PartialPluginSettings {
            grayscale: Some(true),
        });

        if loading {
            let image = render_action(settings.clone(), false).await;
            let mut rotation = 0.0;
            let starting = Instant::now();
            loop {
                sd.set_image_b64(
                    context.clone(),
                    skia_image_to_b64(loading_image(image.clone().unwrap(), rotation, false)),
                )
                .await;
                sleep(Duration::from_millis(33)).await;
                if starting.elapsed().as_secs() > 2 {
                    break;
                }
                rotation = (rotation + 10.0) % 360.0;
            }

            sd.set_image_b64(
                context.clone(),
                skia_image_to_b64(loading_image(image.clone().unwrap(), 0.0, true)),
            )
            .await;

            sleep(Duration::from_millis(2000)).await;
        }

        let grayscale_enabled = global_settings.grayscale.unwrap_or(true);
        let image = render_action(settings.clone(), grayscale_enabled).await;
        sd.set_image_b64(context.clone(), skia_image_to_b64(image))
            .await;
    }

    async fn pull_item(
        &self,
        context: String,
        settings: PullItemSettings,
        sd: StreamDeck,
        equip: bool,
    ) {
        if let Some(item) = settings.item {
            let data = SendPullItem {
                item,
                equip,
                context: context.clone(),
            };
            sd.external(with_action("pullItem", json_string!(&data)))
                .await;
            sd.show_ok(context).await;
        } else {
            sd.show_alert(context).await;
        }
    }
}

#[async_trait]
impl Action for PullItemAction {
    fn uuid(&self) -> &str {
        "com.dim.streamdeck.pull-item"
    }

    fn long_timeout(&self) -> f32 {
        750.0
    }

    async fn on_appear(&self, e: AppearEvent, sd: StreamDeck) {
        let settings: Option<PullItemSettings> = get_settings(e.payload.settings);
        if let Some(settings) = settings {
            self.update(e.context, settings, false, sd).await;
        }
    }

    async fn on_key_up(&self, e: KeyEvent, sd: StreamDeck) {
        let settings: Option<PullItemSettings> = get_settings(e.payload.settings);
        if let Some(settings) = settings {
            self.pull_item(
                e.context.clone(),
                settings.clone(),
                sd.clone(),
                e.is_double_tap && settings.alt_action_trigger == Some("double".to_owned()),
            )
            .await;
            self.update(e.context, settings, true, sd).await;
        }
    }

    async fn on_long_press(&self, e: KeyEvent, _: f32, sd: StreamDeck) {
        let settings: Option<PullItemSettings> = get_settings(e.payload.settings);
        if let Some(settings) = settings {
            if settings.alt_action_trigger == Some("hold".to_owned()) {
                self.pull_item(e.context, settings, sd, true).await;
            }
        }
    }

    async fn on_settings_changed(&self, e: DidReceiveSettingsEvent, sd: StreamDeck) {
        let settings: Option<PullItemSettings> = get_settings(e.payload.settings);
        if let Some(settings) = settings {
            self.update(e.context, settings, false, sd).await
        }
    }

    async fn on_global_settings_changed(&self, _e: DidReceiveGlobalSettingsEvent, sd: StreamDeck) {
        let instances = sd.contexts_of(self.uuid()).await;
        for ctx in instances {
            let settings: Option<PullItemSettings> = sd.settings(ctx.clone()).await;
            self.update(ctx.clone(), settings.unwrap(), false, sd.clone())
                .await
        }
    }

    async fn on_send_to_plugin(&self, e: SendToPluginEvent, sd: StreamDeck) {
        if !e.payload.contains_key("action") {
            return;
        }
        let action = e.payload.get("action").unwrap().as_str().unwrap();
        let id = e.payload.get("id");
        match action {
            "select" => {
                let mut tmp = SHARED.lock().await;
                tmp.insert("item".to_owned(), json!(e.context.clone()));
                let selection = Selection::new("item");
                sd.external(with_action("selection", json_string!(&selection)))
                    .await;
            }
            "show" => {
                let search_field = format!("id:{}", id.unwrap().to_string());
                let search = SearchSettings::new(search_field);
                sd.external(with_action("search", json_string!(&search)))
                    .await;
            }
            &_ => unreachable!(),
        }
    }
}
