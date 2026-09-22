prpr_l10n::tl_file!("home");

use super::{
    load_font_with_cksum, set_bold_font, EventPage, LibraryPage, MessagePage, NextPage, Page, RankedPage, ResPackPage, SFader, SettingsPage, SharedState,
    BOLD_FONT_CKSUM,
};
use crate::{
    anim::Anim,
    client::{recv_raw, Character, Client, ErrorCode, LoginParams, User, UserManager},
    dir, get_data, get_data_mut,
    icons::Icons,
    login::Login,
    save_data,
    scene::{check_read_tos_and_policy, ProfileScene, JUST_LOADED_TOS},
    sync_data,
    threed::ThreeD,
};
use ::rand::{random, thread_rng, Rng};
use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use image::DynamicImage;
use macroquad::prelude::*;
use prpr::{
    core::BOLD_FONT,
    ext::{open_url, screen_aspect, semi_black, semi_white, RectExt, SafeTexture, ScaleType},
    info::ChartInfo,
    scene::{show_error, NextScene},
    task::Task,
    ui::{button_hit_large, clip_rounded_rect, ClipType, DRectButton, Dialog, FontArc, RectButton, Scroll, Ui},
};
use prpr_l10n::LANG_IDENTS;
use reqwest::StatusCode;
use serde::Deserialize;
use std::{
    borrow::Cow,
    sync::{
        atomic::{AtomicI8, Ordering},
        Arc,
    },
};
use tap::Tap;
use tracing::{info, warn};
use lyon::math::point;
use lyon::path::Path;

const BOARD_SWITCH_TIME: f32 = 4.;
const BOARD_TRANSIT_TIME: f32 = 1.2;

type BoldFontUpdateTask = Task<Result<Option<(FontArc, String)>>>;

#[derive(Deserialize)]
struct Version {
    version: semver::Version,
    date: NaiveDate,
    description: String,
    url: String,
}

/// 构造一个平行四边形的 lyon Path。
/// `skew` 是上边相对下边的水平偏移量。
fn parallelogram_path(r: Rect, skew: f32) -> Path {
    let mut builder = Path::builder();
    builder.begin(point(r.x + skew, r.y));
    builder.line_to(point(r.right(), r.y));
    builder.line_to(point(r.right() - skew, r.bottom()));
    builder.line_to(point(r.x, r.bottom()));
    builder.end(true);
    builder.build()
}

pub struct HomePage {
    icons: Arc<Icons>,

    btn_play: DRectButton,
    btn_event: DRectButton,
    btn_respack: DRectButton,
    btn_msg: DRectButton,
    btn_settings: DRectButton,
    btn_user: DRectButton,
    btn_strict: RectButton,

    next_page: Option<NextPage>,

    login: Login,
    update_task: Option<Task<Result<User>>>,
    /// Outcome of the session-restore pending-delete dialog: 0 = none,
    /// 1 = confirm (cancel the deletion and retry), 2 = cancel (log out).
    pending_delete_choice: Arc<AtomicI8>,

    need_back: bool,
    sf: SFader,

    board_task: Option<Task<Result<Option<DynamicImage>>>>,
    board_last_time: f32,
    board_last: Option<String>,
    board_tex_last: Option<SafeTexture>,
    board_tex: Option<SafeTexture>,
    board_dir: bool,

    has_new_task: Option<Task<Result<bool>>>,
    has_new: bool,

    check_update_task: Option<Task<Result<Option<Version>>>>,
    check_bold_font_update_task: Option<BoldFontUpdateTask>,

    btn_play_3d: ThreeD,
    btn_other_3d: ThreeD,

    character: Character,
    char_appear_p: Anim<f32>,
    char_last_illu: Option<String>,
    char_last_user_id: Option<i32>,
    char_fetch_task: Option<Task<Result<Character>>>,
    char_illu: Option<SafeTexture>,
    char_illu_task: Option<Task<Result<DynamicImage>>>,
    // progress of character screen
    char_screen_p: Anim<f32>,
    char_btn: RectButton,
    char_text_start: f32,
    char_cached_size: f32,
    char_scroll: Scroll,
    char_edit_btn: RectButton,

    #[cfg(feature = "hykb")]
    beian_btn: RectButton,
}

impl HomePage {
    pub async fn new(icons: Arc<Icons>) -> Result<Self> {
        let update_task = if get_data().config.offline_mode {
            None
        } else if let Some(u) = &get_data().me {
            UserManager::request(u.id);
            Some(Task::new(async {
                Client::login(LoginParams::RefreshToken {
                    token: &get_data().tokens.as_ref().unwrap().1,
                    cancel_delete_request: false,
                })
                .await?;
                let me = Client::get_me().await?;
                // On HYKB builds a restored session still requires anti-addiction
                // coverage, which is driven by a signed-in native HYKB account.
                // Restore that session silently (no account picker) and tear the
                // in-game session down if the player cancels (`ok_or_err`). The
                // credential is used only for the SDK's online anti-addiction —
                // it is not verified against the restored account, so any
                // successful HYKB login is accepted whether or not the Phira
                // account is bound.
                #[cfg(feature = "hykb")]
                crate::obtain_hykb_credential_silent().await?.ok_or_err()?;
                Ok(me)
            }))
        } else {
            None
        };

        let flavor = match load_file("flavor").await.map(String::from_utf8) {
            Ok(Ok(flavor)) => flavor.trim().to_owned(),
            _ => "none".to_owned(),
        };

        let mut res = Self {
            icons: Arc::clone(&icons),

            btn_play: DRectButton::new().with_delta(-0.01).no_sound(),
            btn_event: DRectButton::new().with_elevation(0.002).no_sound(),
            btn_respack: DRectButton::new().with_elevation(0.002).no_sound(),
            btn_msg: DRectButton::new().with_radius(0.008).with_delta(-0.003).with_elevation(0.002),
            btn_settings: DRectButton::new().with_radius(0.008).with_delta(-0.003).with_elevation(0.002),
            btn_user: DRectButton::new().with_delta(-0.003),
            btn_strict: RectButton::new(),

            next_page: None,

            login: Login::new(icons),
            update_task,
            pending_delete_choice: Arc::new(AtomicI8::new(0)),

            need_back: false,
            sf: SFader::new(),

            board_task: None,
            board_last_time: f32::NEG_INFINITY,
            board_last: None,
            board_tex_last: None,
            board_tex: None,
            board_dir: false,

            has_new_task: None,
            has_new: false,

            check_update_task: Some(Task::new(async move {
                Ok(recv_raw(Client::get("/check-update").query(&[("version", env!("CARGO_PKG_VERSION")), ("flavor", &flavor)]))
                    .await?
                    .json()
                    .await?)
            })),
            check_bold_font_update_task: {
                let cksum = BOLD_FONT_CKSUM.with(|it| it.borrow().clone());
                Some(Task::new(async move {
                    let resp = Client::get("/font-bold")
                        .query(&[("cksum", cksum)])
                        .query(&[("new_bold_font", "true")])
                        .send()
                        .await?;
                    if resp.status() == StatusCode::NOT_MODIFIED {
                        info!("bold font not modified");
                        return Ok(None);
                    }
                    if !resp.status().is_success() {
                        let status = resp.status().as_str().to_owned();
                        let text = resp.text().await.context("failed to receive text")?;
                        if let Ok(what) = serde_json::from_str::<serde_json::Value>(&text) {
                            if let Some(detail) = what["error"].as_str() {
                                bail!("request failed ({status}): {detail}");
                            }
                        }
                        bail!("request failed ({status}): {text}");
                    }
                    info!("downloading new bold font");
                    let bytes = resp.bytes().await?;
                    std::fs::write(dir::bold_font_path()?, &bytes).context("failed to save font")?;
                    Ok(Some(load_font_with_cksum(bytes.to_vec())?))
                }))
            },

            btn_play_3d: ThreeD::new(),
            btn_other_3d: ThreeD::new().tap_mut(|it| {
                it.anchor = vec2(0.2, -0.2);
                it.angle = 0.14;
                it.sync();
            }),

            character: get_data().character.clone().unwrap_or_default(),
            char_appear_p: Anim::new(0.),
            char_last_illu: None,
            char_last_user_id: None,
            char_fetch_task: None,
            char_illu: None,
            char_illu_task: None,
            char_screen_p: Anim::new(0.),
            char_btn: RectButton::new(),
            char_text_start: 0.,
            char_cached_size: 0.,
            char_scroll: Scroll::new().use_clip(ClipType::Clip),
            char_edit_btn: RectButton::new(),

            #[cfg(feature = "hykb")]
            beian_btn: RectButton::new(),
        };
        res.load_char_illu();

        Ok(res)
    }
}

impl HomePage {
    fn load_char_illu(&mut self) {
        let key = if self.character.illust == "@" {
            format!("@{}", self.character.id)
        } else {
            self.character.illust.clone()
        };
        if self.char_last_illu.as_ref() == Some(&key) {
            return;
        }
        self.char_last_illu = Some(key);

        self.char_appear_p.set(0.);

        #[cfg(closed)]
        if self.character.illust == "@" {
            let id = self.character.id.clone();
            self.char_illu_task =
                Some(Task::new(
                    async move { Ok(image::load_from_memory(&crate::inner::resolve_data(load_file(&format!("res/{id}.char")).await?))?) },
                ));
        } else {
            let file = crate::page::File {
                url: self.character.illust.clone(),
            };
            self.char_illu_task =
                Some(Task::new(async move { Ok(image::load_from_memory(&crate::inner::resolve_data(file.fetch().await?.to_vec()))?) }));
        }
    }

    fn fetch_has_new(&mut self) {
        if get_data().config.offline_mode || get_data().me.is_none() || get_data().tokens.is_none() {
            self.has_new_task = None;
            self.has_new = false;
            return;
        }
        let time = get_data().message_check_time.unwrap_or_default();
        self.has_new_task = Some(Task::new(async move {
            #[derive(Deserialize)]
            struct Resp {
                has: bool,
            }
            let resp: Resp = recv_raw(Client::get("/message/has_new").query(&[("checked", time)]))
                .await?
                .json()
                .await?;
            Ok(resp.has)
        }));
    }

    fn render_not_char(&mut self, ui: &mut Ui, s: &mut SharedState) {
        let screen = ui.screen_rect();

        // ============ 顶部：标题 ============
    ui.text("Phira.fozu")
    .pos(screen.center().x, screen.y + 0.05)
    .anchor(0.5, 0.0)
    .size(1.2)
    .color(WHITE)
    .draw();

        // ============ 中间：立绘由 char_scroll 处理 ============
        // 无需改

        // ============ 底部：一整条平行四边形按钮组 ============
        let bar_r = Rect::new(screen.x + 0.05, screen.bottom() - 0.32, screen.w - 0.1, 0.22);
        ui.fill_path(&parallelogram_path(bar_r, 0.05), semi_black(0.55));

        let cell_count = 5;
        let cell_w = bar_r.w / cell_count as f32;

        // 分隔线
        for i in 1..cell_count {
            let x = bar_r.x + cell_w * i as f32;
            ui.fill_rect(
                Rect::new(x - 0.001, bar_r.y + 0.03, 0.002, bar_r.h - 0.06),
                semi_white(0.15),
            );
        }

        // 0: 游玩
        {
            let r = Rect::new(bar_r.x, bar_r.y, cell_w, bar_r.h);
            s.render_fader(ui, |ui| {
                let icon_r = Rect::new(r.center().x - 0.06, r.center().y - 0.08, 0.12, 0.12);
                ui.fill_rect(icon_r, (*self.icons.play, icon_r, ScaleType::Fit, WHITE));
                ui.text(tl!("play"))
                    .pos(r.center().x, r.bottom() - 0.045)
                    .anchor(0.5, 0.0)
                    .size(0.45)
                    .color(semi_white(0.85))
                    .draw();
            });
            self.btn_play.build(ui, s.t, r, |_ui, _| {});
        }

        // 1: 活动
        {
            let r = Rect::new(bar_r.x + cell_w, bar_r.y, cell_w, bar_r.h);
            s.render_fader(ui, |ui| {
                let icon_r = Rect::new(r.center().x - 0.05, r.center().y - 0.08, 0.1, 0.1);
                ui.fill_rect(icon_r, (*self.icons.medal, icon_r, ScaleType::Fit, semi_white(0.85)));
                ui.text("段位认证")
                    .pos(r.center().x, r.bottom() - 0.045)
                    .anchor(0.5, 0.0)
                    .size(0.4)
                    .color(semi_white(0.7))
                    .draw();
            });
            self.btn_event.build(ui, s.t, r, |_ui, _| {});
        }

        // 2: 资源包
        {
            let r = Rect::new(bar_r.x + cell_w * 2., bar_r.y, cell_w, bar_r.h);
            s.render_fader(ui, |ui| {
                let icon_r = Rect::new(r.center().x - 0.05, r.center().y - 0.08, 0.1, 0.1);
                ui.fill_rect(icon_r, (*self.icons.respack, icon_r, ScaleType::Fit, semi_white(0.85)));
                ui.text(tl!("respack"))
                    .pos(r.center().x, r.bottom() - 0.045)
                    .anchor(0.5, 0.0)
                    .size(0.4)
                    .color(semi_white(0.7))
                    .draw();
            });
            self.btn_respack.build(ui, s.t, r, |_ui, _| {});
        }

        // 3: 消息
        {
            let r = Rect::new(bar_r.x + cell_w * 3., bar_r.y, cell_w, bar_r.h);
            s.render_fader(ui, |ui| {
                let icon_r = Rect::new(r.center().x - 0.05, r.center().y - 0.08, 0.1, 0.1);
                ui.fill_rect(icon_r, (*self.icons.msg, icon_r, ScaleType::Fit, semi_white(0.85)));
                if self.has_new {
                    ui.fill_circle(icon_r.right(), icon_r.y, 0.012, RED);
                }
                ui.text("消息")
                    .pos(r.center().x, r.bottom() - 0.045)
                    .anchor(0.5, 0.0)
                    .size(0.4)
                    .color(semi_white(0.7))
                    .draw();
            });
            self.btn_msg.build(ui, s.t, r, |_ui, _| {});
        }

        // 4: 设置
        {
            let r = Rect::new(bar_r.x + cell_w * 4., bar_r.y, cell_w, bar_r.h);
            s.render_fader(ui, |ui| {
                let icon_r = Rect::new(r.center().x - 0.05, r.center().y - 0.08, 0.1, 0.1);
                ui.fill_rect(icon_r, (*self.icons.settings, icon_r, ScaleType::Fit, semi_white(0.85)));
                ui.text("设置")
                    .pos(r.center().x, r.bottom() - 0.045)
                    .anchor(0.5, 0.0)
                    .size(0.4)
                    .color(semi_white(0.7))
                    .draw();
            });
            self.btn_settings.build(ui, s.t, r, |_ui, _| {});
        }

                // ============ 严判模式开关（右侧，按钮条上方） ============
        let strict_on = prpr::judge::STRICT_MODE.load(std::sync::atomic::Ordering::Relaxed);
        // 位置：屏幕右侧，距右边 0.05；竖直方向在按钮条上方
        let strict_btn_r = Rect::new(
            screen.right() - 0.47,          // 右边 0.47（含宽度）
            screen.bottom() - 0.55,          // 比按钮条高
            0.42,                            // 宽度（够放"严判: 开"）
            0.10,                            // 高度
        );

        let bg_color = if strict_on {
            Color::new(1.0, 0.45, 0.2, 0.9)
        } else {
            semi_black(0.55)
        };
        ui.fill_path(&parallelogram_path(strict_btn_r, 0.02), bg_color);

        let label = if strict_on { "严判: 开" } else { "严判: 关" };
        ui.text(label)
            .pos(strict_btn_r.center().x, strict_btn_r.center().y)
            .anchor(0.5, 0.5)
            .size(0.38)                      // 小一点
            .color(WHITE)
            .draw();

        self.btn_strict.set(ui, strict_btn_r);
    }
}

impl Page for HomePage {
    fn label(&self) -> Cow<'static, str> {
    "".into()
}

    fn enter(&mut self, s: &mut SharedState) -> Result<()> {
        if self.need_back {
            self.sf.enter(s.t);
            self.need_back = false;
        }
        self.fetch_has_new();
        Ok(())
    }

    fn touch(&mut self, touch: &Touch, s: &mut SharedState) -> Result<bool> {
        if self.sf.transiting() {
            return Ok(true);
        }
        // The HYKB startup check (refresh + verify SDK session) is blocking: keep
        // the home page inert until it resolves.
        #[cfg(feature = "hykb")]
        if self.update_task.is_some() {
            return Ok(true);
        }
        let t = s.t;
        let rt = s.rt;
        if self.login.touch(touch, s.t) {
            return Ok(true);
        }
        if self.char_screen_p.now(rt) < 1e-2 {
            self.btn_play_3d.touch(touch, t);
            if self.btn_play.touch(touch, t) {
                button_hit_large();
                self.next_page = Some(NextPage::Overlay(Box::new(LibraryPage::new(Arc::clone(&self.icons), s.icons.clone())?)));
                return Ok(true);
            }
            if self.btn_event.touch(touch, t) {
                button_hit_large();
                self.next_page = Some(NextPage::Overlay(Box::new(RankedPage::new(Arc::clone(&self.icons)))));
                return Ok(true);
            }
            if self.btn_respack.touch(touch, t) {
                button_hit_large();
                self.next_page = Some(NextPage::Overlay(Box::new(ResPackPage::new(Arc::clone(&self.icons))?)));
                return Ok(true);
            }
            if self.btn_msg.touch(touch, t) {
                if check_read_tos_and_policy(true, true) {
                    self.next_page = Some(NextPage::Overlay(Box::new(MessagePage::new(Arc::clone(&self.icons), s.icons.clone()))));
                }
                return Ok(true);
            }
            if self.btn_settings.touch(touch, t) {
                self.next_page = Some(NextPage::Overlay(Box::new(SettingsPage::new(self.icons.icon.clone(), self.icons.lang.clone()))));
                return Ok(true);
            }
        } else {
            if self.char_scroll.touch(touch, t) {
                return Ok(true);
            }
            if self.char_edit_btn.touch(touch) {
                let _ = open_url("https://phira.moe/settings/account");
            }
        }
        if self.btn_user.touch(touch, t) {
            if let Some(me) = &get_data().me {
                self.need_back = true;
                self.sf.goto(t, ProfileScene::new(me.id, self.icons.user.clone(), s.icons.clone()));
            } else {
                self.login.enter(t);
            }
            return Ok(true);
        }
        if self.btn_strict.touch(touch) {
            let was = prpr::judge::STRICT_MODE.load(std::sync::atomic::Ordering::Relaxed);
            prpr::judge::STRICT_MODE.store(!was, std::sync::atomic::Ordering::Relaxed);
            info!("[strict] 严判模式: {}", if !was { "开" } else { "关" });
            return Ok(true);
        }
        #[cfg(feature = "hykb")]
        if self.beian_btn.touch(touch) {
            let _ = open_url("https://beian.miit.gov.cn/#/home");
            return Ok(true);
        }
        if self.char_btn.touch(touch) {
            if !self.char_screen_p.transiting(rt) {
                let to = if self.char_screen_p.now(rt) < 0.5 {
                    self.char_text_start = rt;
                    1.
                } else {
                    0.
                };
                self.char_screen_p.goto(to, rt, 0.5);
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn update(&mut self, s: &mut SharedState) -> Result<()> {
        let t = s.t;
        // HYKB builds require an account: while signed out, keep the login panel
        // forced open. Polling here (rather than only on entry) also covers the
        // player manually logging out and popping back to the home page.
        #[cfg(feature = "hykb")]
        if get_data().me.is_none() {
            self.login.force(t);
        }
        self.login.update(t)?;
        let current_user = Some(get_data().me.as_ref().map_or(-1, |it| it.id));
        self.char_scroll.update(t);
        if self.char_last_user_id != current_user {
            let locale = get_data().language.clone().unwrap_or(LANG_IDENTS[0].to_string());
            self.char_last_user_id = current_user;
            if get_data().config.offline_mode || get_data().me.is_none() || get_data().tokens.is_none() {
                self.char_fetch_task = None;
            } else {
                self.char_fetch_task =
                    Some(Task::new(async move { Ok(recv_raw(Client::get("/me/char").query(&[("locale", locale)])).await?.json().await?) }));
            }
        }
        if let Some(task) = &mut self.update_task {
            if let Some(res) = task.take() {
                match res {
                    Err(err) => {
                        // wtf bro
                        if format!("{err:?}").contains("invalid token") {
                            get_data_mut().me = None;
                            get_data_mut().tokens = None;
                            let _ = save_data();
                            sync_data();
                        }
                        if err.downcast_ref::<ErrorCode>() == Some(&ErrorCode::PENDING_DELETE_REQUEST) {
                            self.pending_delete_choice.store(0, Ordering::SeqCst);
                            use crate::login::{tl as ltl, L10N_LOCAL};
                            let choice = Arc::clone(&self.pending_delete_choice);
                            Dialog::plain(ltl!("pending-delete-title").into_owned(), ltl!("pending-delete-message").into_owned())
                                .buttons(vec![ttl!("cancel").into_owned(), ttl!("confirm").into_owned()])
                                .listener(move |_dialog, id| {
                                    if id == -1 {
                                        return true;
                                    }
                                    choice.store(if id == 1 { 1 } else { 2 }, Ordering::SeqCst);
                                    false
                                })
                                .show();
                        } else {
                            // TODO: better error handling
                            show_error(err.context(tl!("failed-to-update") + "\n" + tl!("note-try-login-again")));
                        }
                    }
                    Ok(val) => {
                        get_data_mut().me = Some(val);
                        save_data()?;
                    }
                }
                self.update_task = None;
            }
        }
        match self.pending_delete_choice.swap(0, Ordering::Relaxed) {
            1 => {
                let tokens = get_data().tokens.clone();
                self.update_task = tokens.map(|(_, refresh)| {
                    Task::new(async move {
                        Client::login(LoginParams::RefreshToken {
                            token: &refresh,
                            cancel_delete_request: true,
                        })
                        .await?;
                        let me = Client::get_me().await?;
                        #[cfg(feature = "hykb")]
                        crate::obtain_hykb_credential_silent().await?.ok_or_err()?;
                        Ok(me)
                    })
                });
            }
            2 => {
                crate::force_logout();
            }
            _ => {}
        }
        if self.board_task.is_none() && t - self.board_last_time > BOARD_SWITCH_TIME {
            let charts = &get_data().charts;
            let last_index = self
                .board_last
                .as_ref()
                .and_then(|path| charts.iter().position(|it| &it.local_path == path));
            if charts.is_empty() || (charts.len() == 1 && last_index.is_some()) {
                self.board_task = Some(Task::new(async move { Ok(None) }));
            } else {
                let mut index = thread_rng().gen_range(0..(charts.len() - last_index.is_some() as usize));
                if last_index.is_some_and(|it| it <= index) {
                    index += 1;
                }
                let path = charts[index].local_path.clone();
                let dir = prpr::dir::Dir::new(format!("{}/{}", dir::charts()?, path))?;
                self.board_last = Some(path);
                self.board_task = Some(Task::new(async move {
                    let info: ChartInfo = serde_yaml::from_reader(dir.open("info.yml")?)?;
                    let bytes = dir.read(info.illustration)?;
                    Ok(Some(image::load_from_memory(&bytes)?))
                }));
            }
        }
        if let Some(task) = &mut self.board_task {
            if let Some(res) = task.take() {
                match res {
                    Err(err) => {
                        warn!(?err, "failed to load illustration for board");
                    }
                    Ok(image) => {
                        if let Some(image) = image {
                            let tex: SafeTexture = image.into();
                            self.board_tex_last = self.board_tex.replace(tex);
                            self.board_dir = random();
                        }
                    }
                }
                self.board_last_time = t;
                self.board_task = None;
            }
        }
        if let Some(task) = &mut self.has_new_task {
            if let Some(res) = task.take() {
                match res {
                    Err(err) => {
                        warn!("fail to load has new {:?}", err);
                    }
                    Ok(has) => {
                        self.has_new = has;
                    }
                }
                self.has_new_task = None;
            }
        }
        if let Some(task) = &mut self.check_update_task {
            if let Some(res) = task.take() {
                match res {
                    Err(err) => {
                        warn!("fail to check update {:?}", err);
                    }
                    Ok(Some(ver)) => {
                        if get_data().ignored_version.as_ref().is_none_or(|it| it < &ver.version) {
                            Dialog::plain(
                                tl!("update", "version" => ver.version.to_string()),
                                tl!("update-desc", "date" => ver.date.to_string(), "desc" => ver.description),
                            )
                            .buttons(vec![
                                ttl!("cancel").into_owned(),
                                tl!("update-ignore").into_owned(),
                                tl!("update-go").into_owned(),
                            ])
                            .listener(move |_dialog, pos| {
                                match pos {
                                    1 => {
                                        get_data_mut().ignored_version = Some(ver.version.clone());
                                        let _ = save_data();
                                    }
                                    2 => {
                                        let _ = open_url(&ver.url);
                                    }
                                    _ => {}
                                }
                                false
                            })
                            .show();
                        }
                    }
                    _ => {}
                }
                self.check_update_task = None;
            }
        }
        if let Some(task) = &mut self.check_bold_font_update_task {
            if let Some(res) = task.take() {
                match res {
                    Err(err) => {
                        warn!("fail to check bold font update {:?}", err);
                    }
                    Ok(None) => {}
                    Ok(Some(parsed)) => {
                        info!(cksum = parsed.1, "new bold font");
                        set_bold_font(parsed);
                    }
                }
                self.check_bold_font_update_task = None;
            }
        }
        if let Some(task) = &mut self.char_illu_task {
            if let Some(res) = task.take() {
                match res {
                    Err(err) => {
                        warn!(?err, "fail to load char illu");
                    }
                    Ok(image) => {
                        self.char_appear_p.goto(1., t, 0.5);
                        let tex: SafeTexture = image.into();
                        self.char_illu = Some(tex.with_mipmap());
                    }
                }
                self.char_illu_task = None;
            }
        }
        if let Some(task) = &mut self.char_fetch_task {
            if let Some(res) = task.take() {
                match res {
                    Err(err) => {
                        warn!(?err, "fail to load char");
                    }
                    Ok(char) => {
                        info!(?char, "char loaded");
                        self.character = char;
                        get_data_mut().character = Some(self.character.clone());
                        let _ = save_data();
                        self.char_cached_size = 0.;
                        self.load_char_illu();
                    }
                }
                self.char_fetch_task = None;
            }
        }
        if JUST_LOADED_TOS.fetch_and(false, Ordering::Relaxed) {
            check_read_tos_and_policy(true, true);
        }

        Ok(())
    }

    fn render(&mut self, ui: &mut Ui, s: &mut SharedState) -> Result<()> {
        let t = s.t;
        let rt = s.rt;

        let cp = self.char_screen_p.now(rt);
        s.render_fader(ui, |ui| {
            let r = Rect::new(-1. + 0.14 * cp, -ui.top + 0.12, 1., 1.7);
            if let Some(illu) = &self.char_illu {
                let p = self.char_appear_p.now(t);
                let (ox, oy, ow, oh) = self.character.illu_adjust;
                let r = Rect::new(r.x + ox, r.y + (1. - p) * 0.05 + oy, r.w + ow, r.h + oh);
                ui.fill_rect(ui.screen_rect(), (**illu, r, ScaleType::CropCenter, semi_white(p)));
            }
            self.char_btn.set(ui, r);

            if cp > 1e-5 {
                let height = 0.8 - ((screen_aspect() - 16. / 9.) * 0.2).min(0.2);
                let r = Rect::new(0.16, (-height - height * cp) / 4., 0.6, height);
                let mat = ThreeD::build(vec2(0., 0.), r, 0.12);
                let gl = unsafe { get_internal_gl() }.quad_gl;
                gl.push_model_matrix(mat);

                ui.alpha(cp, |ui| {
                    let mut r = Rect::new(r.x, r.y + 0.14, r.w, r.h - 0.14);
                    ui.fill_rect(r, semi_black(0.3));
                    ui.fill_rect(Rect::new(r.x, r.y, 0.01, r.h), WHITE);
                    let mut t = ui.text(tl!("change-char")).pos(r.x + 0.01, r.bottom() + 0.015).size(0.3);
                    let ir = t.measure().feather(0.007);
                    t.ui.fill_rect(ir, semi_black(0.2));
                    self.char_edit_btn.set(t.ui, ir);
                    t.draw();
                    let pad = 0.01;

                    let mut t = ui
                        .text(self.character.name_en())
                        .pos(r.right() - pad, r.bottom() - pad)
                        .anchor(1., 1.)
                        .color(semi_white(0.2));
                    if self.char_cached_size < 1e-6 {
                        let mut initial = 2.;
                        loop {
                            t = t.size(initial);
                            if t.measure().w < r.w * 0.7 {
                                break;
                            }
                            initial *= 0.95;
                        }
                        self.char_cached_size = initial;
                    } else {
                        t = t.size(self.char_cached_size);
                    }
                    t.draw();

                    r.x += 0.01;
                    r.w -= 0.01;

                    self.char_scroll.size((r.w, r.h));
                    ui.scope(|ui| {
                        ui.dx(r.x);
                        ui.dy(r.y);
                        let ow = r.w;
                        self.char_scroll.render(ui, |ui| {
                            let r = Rect::new(0., 0., r.w, r.h);
                            let r = r.feather(-0.03);
                            let r = ui.text(&self.character.intro).pos(r.x, r.y).max_width(r.w).multiline().size(0.4).draw();
                            (ow, r.h + 0.1)
                        });
                    });
                });

                let r = Rect::new(r.x, r.y, 0.4, 0.12);

                ui.alpha(cp, |ui| {
                    let r = ui
                        .text(&self.character.name)
                        .pos(r.x + (1. - cp) * 0.12 + 0.01, r.center().y)
                        .anchor(0., 0.5)
                        .size(self.character.name_size.unwrap_or(1.4))
                        .draw_using(&BOLD_FONT);

                    let off = if self.character.baseline { 0. } else { 0.01 };
                    ui.text(format!("Artist: {}", self.character.artist))
                        .pos(r.right() + (1. - cp) * 0.1 + 0.02, r.bottom() + off - 0.03)
                        .anchor(0., 1.)
                        .size(0.34)
                        .color(semi_white(0.7))
                        .draw();
                    ui.text(format!("Designer: {}", self.character.designer))
                        .pos(r.right() + (1. - cp) * 0.1 + 0.016, r.bottom() + off)
                        .anchor(0., 1.)
                        .size(0.34)
                        .color(semi_white(0.7))
                        .draw();
                });

                gl.pop_model_matrix();
            }
        });

        ui.alpha(1. - cp, |ui| {
            self.render_not_char(ui, s);
        });

        s.fader.roll_back();
        s.render_fader(ui, |ui| {
            let rad = 0.05;
            let ct = (0.92, -ui.top + 0.08);
            self.btn_user.config.radius = rad;
            let r = Rect::new(ct.0, ct.1, 0., 0.).feather(rad);
            self.btn_user.build(ui, t, r, |ui, _| {
                ui.avatar(
                    ct.0,
                    ct.1,
                    r.w / 2.,
                    t,
                    get_data()
                        .me
                        .as_ref()
                        .map(|user| UserManager::opt_avatar(user.id, &self.icons.user))
                        .unwrap_or(Err(self.icons.user.clone())),
                );
            });
            let rt = ct.0 - rad - 0.02;
            if let Some(me) = &get_data().me {
                ui.text(&me.name).pos(rt, r.center().y + 0.002).anchor(1., 1.).size(0.6).draw();
                ui.text(format!("PP {:.2}", me.rks))
                    .pos(rt, r.center().y + 0.008)
                    .anchor(1., 0.)
                    .size(0.4)
                    .color(semi_white(0.6))
                    .draw();
            } else {
                ui.text(tl!("not-logged-in"))
                    .pos(rt, r.center().y)
                    .anchor(1., 0.5)
                    .no_baseline()
                    .size(0.6)
                    .draw();
            }

            #[cfg(feature = "hykb")]
            {
                let r = ui.screen_rect();
                let r = ui
                    .text("备案号：闽ICP备18008307号-64A")
                    .pos(r.x + 0.02, r.bottom() - 0.03)
                    .size(0.5)
                    .anchor(0., 1.)
                    .draw();
                self.beian_btn.set(ui, r);
            }
        });

        self.login.render(ui, t);
        // Cover the home page with a blocking loader during the HYKB startup check.
        #[cfg(feature = "hykb")]
        if self.update_task.is_some() {
            ui.full_loading_simple(t);
        }
        self.sf.render(ui, t);

        Ok(())
    }

    fn next_page(&mut self) -> NextPage {
        self.next_page.take().unwrap_or_default()
    }

    fn next_scene(&mut self, s: &mut SharedState) -> NextScene {
        self.sf.next_scene(s.t).unwrap_or_default()
    }
}
