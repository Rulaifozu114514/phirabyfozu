// phira/src/page/ranked.rs
use super::{NextPage, Page, SharedState};
use anyhow::Result;
use macroquad::prelude::*;
use prpr::{
    core::BOLD_FONT,
    ext::{semi_black, semi_white},
    scene::NextScene,
    ui::{RectButton, Ui},
};
use std::{borrow::Cow, sync::Arc};
use crate::icons::Icons;

pub struct RankedPage {
    icons: Arc<Icons>,
    btn_back: RectButton,
    next_page: Option<NextPage>,
}

impl RankedPage {
    pub fn new(icons: Arc<Icons>) -> Self {
        Self {
            icons,
            btn_back: RectButton::new(),
            next_page: None,
        }
    }
}

impl Page for RankedPage {
    fn label(&self) -> Cow<'static, str> {
        "段位认证".into()
    }

    fn touch(&mut self, touch: &Touch, _s: &mut SharedState) -> Result<bool> {
        // 点左上角返回按钮 → 关闭当前页面
        if self.btn_back.touch(touch) {
            self.next_page = Some(NextPage::Pop);
            return Ok(true);
        }
        Ok(false)
    }

    fn update(&mut self, _s: &mut SharedState) -> Result<()> {
        // 临时测试：进段位页就设段位 #1
        crate::scene::CURRENT_RANKED_ID.store(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    fn render(&mut self, ui: &mut Ui, _s: &mut SharedState) -> Result<()> {
        let screen = ui.screen_rect();

        // 半透明黑底
        ui.fill_rect(screen, semi_black(0.85));

        // 大标题
        ui.text("段位认证")
            .pos(screen.center().x, screen.y + 0.20)
            .anchor(0.5, 0.5)
            .size(2.0)
            .color(WHITE)
            .draw_using(&BOLD_FONT);

        // 占位文字
        ui.text("开发中...")
            .pos(screen.center().x, screen.center().y)
            .anchor(0.5, 0.5)
            .size(0.8)
            .color(semi_white(0.5))
            .draw();

        // 左上角返回按钮
        let r = Rect::new(screen.x + 0.05, screen.y + 0.05, 0.15, 0.15);
        self.btn_back.set(ui, r);
        Ok(())
    }

    fn next_page(&mut self) -> NextPage {
        self.next_page.take().unwrap_or_default()
    }

    fn next_scene(&mut self, _s: &mut SharedState) -> NextScene {
        NextScene::None
    }
}