//! Bandeja do sistema (indicador de gravando) e atalho global.
//! Linux: StatusNotifierItem por D-Bus (`ksni`, sem GTK). Windows: `tray-icon`.

use std::sync::mpsc::{channel, Receiver};

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    Show,
    ToggleRecord,
    Quit,
}

pub const HOTKEY_TEXT: &str = "Ctrl+Alt+R";

/// Ícone 32×32 RGBA: disco azul parado, vermelho gravando.
fn icon_rgba(rec: bool) -> Vec<u8> {
    let (r, g, b) = if rec { (229u8, 72u8, 77u8) } else { (79, 140, 255) };
    let mut px = Vec::with_capacity(32 * 32 * 4);
    for y in 0..32 {
        for x in 0..32 {
            let d = (((x as f32 - 15.5).powi(2) + (y as f32 - 15.5).powi(2)).sqrt() - 14.0).clamp(0.0, 1.0);
            px.extend_from_slice(&[r, g, b, ((1.0 - d) * 255.0) as u8]);
        }
    }
    px
}

pub struct Tray {
    rx: Receiver<Cmd>,
    inner: imp::Inner,
    _hotkey: Option<GlobalHotKeyManager>,
    pub hotkey_ok: bool,
}

impl Tray {
    /// `None` se o sistema não tem bandeja (o app segue sem ela e fechar a janela encerra).
    pub fn new() -> Option<Tray> {
        let (tx, rx) = channel();
        let inner = imp::Inner::new(tx)?;
        let mgr = GlobalHotKeyManager::new().ok();
        let hotkey_ok = mgr.as_ref().map(|m| m.register(HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyR)).is_ok()).unwrap_or(false);
        Some(Tray { rx, inner, _hotkey: mgr, hotkey_ok })
    }

    pub fn set_recording(&self, rec: bool) {
        self.inner.set_recording(rec);
    }

    /// Comandos pendentes da bandeja e do atalho.
    pub fn poll(&self) -> Vec<Cmd> {
        let mut v: Vec<Cmd> = self.rx.try_iter().collect();
        self.inner.poll(&mut v);
        while let Ok(e) = GlobalHotKeyEvent::receiver().try_recv() {
            if e.state == HotKeyState::Pressed {
                v.push(Cmd::ToggleRecord);
            }
        }
        v
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use ksni::blocking::{Handle, TrayMethods};
    use ksni::menu::StandardItem;
    use std::sync::mpsc::Sender;

    struct T {
        tx: Sender<Cmd>,
        rec: bool,
    }

    impl ksni::Tray for T {
        fn id(&self) -> String {
            "renanmeet".into()
        }
        fn title(&self) -> String {
            if self.rec { "RenanMeet — gravando" } else { "RenanMeet" }.into()
        }
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            // ARGB32
            let data = icon_rgba(self.rec).chunks_exact(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect();
            vec![ksni::Icon { width: 32, height: 32, data }]
        }
        fn activate(&mut self, _: i32, _: i32) {
            let _ = self.tx.send(Cmd::Show);
        }
        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            let item = |label: &str, cmd: Cmd| {
                StandardItem { label: label.into(), activate: Box::new(move |t: &mut T| { let _ = t.tx.send(cmd); }), ..Default::default() }.into()
            };
            vec![
                item("Abrir RenanMeet", Cmd::Show),
                item(if self.rec { "Parar gravação" } else { "Gravar reunião…" }, Cmd::ToggleRecord),
                item("Sair", Cmd::Quit),
            ]
        }
    }

    pub struct Inner(Handle<T>);

    impl Inner {
        pub fn new(tx: Sender<Cmd>) -> Option<Self> {
            T { tx, rec: false }.spawn().ok().map(Inner)
        }
        pub fn set_recording(&self, rec: bool) {
            self.0.update(|t| t.rec = rec);
        }
        pub fn poll(&self, _: &mut Vec<Cmd>) {}
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::sync::mpsc::Sender;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    pub struct Inner {
        icon: TrayIcon,
        open: MenuItem,
        rec: MenuItem,
        quit: MenuItem,
    }

    fn icon(rec: bool) -> Option<Icon> {
        Icon::from_rgba(icon_rgba(rec), 32, 32).ok()
    }

    impl Inner {
        pub fn new(_: Sender<Cmd>) -> Option<Self> {
            let (open, rec, quit) = (MenuItem::new("Abrir RenanMeet", true, None), MenuItem::new("Gravar reunião…", true, None), MenuItem::new("Sair", true, None));
            let menu = Menu::new();
            menu.append_items(&[&open, &rec, &quit]).ok()?;
            let icon = TrayIconBuilder::new().with_menu(Box::new(menu)).with_menu_on_left_click(false).with_tooltip("RenanMeet").with_icon(icon(false)?).build().ok()?;
            Some(Inner { icon, open, rec, quit })
        }
        pub fn set_recording(&self, on: bool) {
            if let Some(i) = icon(on) {
                let _ = self.icon.set_icon(Some(i));
            }
            self.rec.set_text(if on { "Parar gravação" } else { "Gravar reunião…" });
            let _ = self.icon.set_tooltip(Some(if on { "RenanMeet — gravando" } else { "RenanMeet" }));
        }
        pub fn poll(&self, out: &mut Vec<Cmd>) {
            while let Ok(e) = MenuEvent::receiver().try_recv() {
                if e.id == *self.open.id() {
                    out.push(Cmd::Show);
                } else if e.id == *self.rec.id() {
                    out.push(Cmd::ToggleRecord);
                } else if e.id == *self.quit.id() {
                    out.push(Cmd::Quit);
                }
            }
            while let Ok(e) = TrayIconEvent::receiver().try_recv() {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                    out.push(Cmd::Show);
                }
            }
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod imp {
    use super::*;
    use std::sync::mpsc::Sender;
    pub struct Inner;
    impl Inner {
        pub fn new(_: Sender<Cmd>) -> Option<Self> {
            None
        }
        pub fn set_recording(&self, _: bool) {}
        pub fn poll(&self, _: &mut Vec<Cmd>) {}
    }
}
