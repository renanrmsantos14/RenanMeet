//! RenanMeet (desktop): grava, mostra a reunião e chama o processamento em outro processo,
//! para que a RAM da análise seja devolvida ao sistema assim que ela termina.

mod player;
mod sys;
mod tray;

use std::cell::RefCell;
use std::collections::HashMap;
use std::process::{Child, Command};
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use meet_core::audio::decode::{load_16k, Channel};
use meet_core::audio::recorder::{Mode, Recorder, RecorderConfig};
use meet_core::clock::br_datetime;
use meet_core::db::{Db, Meeting, Minutes, Segment, Speaker, Stage};
use meet_core::export::{self, Full};
use meet_core::paths::{data_dir, db_path, meeting_dir, models_dir};
use meet_core::pipeline::{self, ProcessConfig, Step};
use meet_core::voice::analyze::enroll_embedding;
use meet_core::voice::sherpa::SherpaPaths;
use meet_core::voice::store::VoiceStore;
use meet_core::{models, secrets};
use slint::{Color, ComponentHandle, Model, ModelRc, SharedString, VecModel, Weak};

slint::include_modules!();

const VERSION: &str = env!("CARGO_PKG_VERSION");
const STAMP: &str = env!("RM_STAMP");
const ENROLL_SECS: u64 = 20;
const LGPD: &str = "Esta reunião está sendo gravada e transcrita para gerar a ata. O áudio e o texto ficam guardados no computador de quem está gravando. Se alguém não concordar, é só avisar: a gravação é interrompida.";
const PALETTE: [(u8, u8, u8); 6] = [(79, 140, 255), (48, 164, 108), (245, 166, 35), (229, 72, 77), (155, 114, 255), (0, 184, 212)];

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("process") {
        return process_headless(args.get(2).ok_or_else(|| anyhow!("uso: renanmeet process <id>"))?);
    }
    run_ui()
}

// ---------- processamento (outro processo) ----------

fn progress_key(id: &str) -> String {
    format!("progress:{id}")
}

fn process_headless(id: &str) -> Result<()> {
    let mut db = Db::open(&db_path())?;
    let res = (|| -> Result<()> {
        let key = secrets::groq_key()?.ok_or_else(|| anyhow!("falta a chave da Groq (Ajustes)"))?;
        let note = |t: &str| {
            let _ = Db::open(&db_path()).and_then(|d| d.set_setting(&progress_key(id), t));
        };
        if !models::missing(&models_dir()).is_empty() {
            let mut last = 0;
            models::ensure(&models_dir(), |name, done, total| {
                let pct = if total > 0 { done * 100 / total } else { 0 };
                if pct != last {
                    last = pct;
                    note(&format!("baixando {name} {pct}%"));
                }
            })?;
        }
        let mut cfg = ProcessConfig::new(key);
        if let Some(t) = db.setting("voice_threshold")?.and_then(|s| s.parse::<f32>().ok()) {
            cfg.analyze.voice_threshold = t;
        }
        let voices = VoiceStore::open(&db_path())?;
        pipeline::process(&mut db, id, &cfg, &voices, &SherpaPaths::locate()?, |s| {
            note(&match s {
                Step::Loading => "carregando áudio".into(),
                Step::Transcribing(d, t) => format!("transcrevendo {d}/{t}"),
                Step::Minutes(d, t) => format!("escrevendo a ata {d}/{t}"),
                Step::Saving => "salvando".into(),
            })
        })
    })();
    let _ = db.set_setting(&progress_key(id), "");
    if let Err(e) = res {
        let _ = db.set_stage(id, Stage::Failed, Some(&format!("{e:#}")));
        return Err(e);
    }
    Ok(())
}

// ---------- estado da interface ----------

struct Current {
    meeting: Meeting,
    segments: Vec<Segment>,
    speakers: Vec<Speaker>,
    minutes: Option<Minutes>,
}

enum Msg {
    Imported(Result<Meeting, String>),
    Enrolled(Result<(), String>),
}

struct Active {
    rec: Recorder,
    id: String,
}

struct Enroll {
    rec: Recorder,
    path: std::path::PathBuf,
    started: Instant,
}

struct Ctx {
    ui: Weak<App>,
    db: Db,
    voices: VoiceStore,
    rec: Option<Active>,
    enroll: Option<Enroll>,
    player: player::Player,
    player_id: Option<String>,
    children: HashMap<String, Child>,
    cur: Option<Current>,
    lines: Rc<VecModel<Line>>,
    active_line: Option<usize>,
    toast_until: Option<Instant>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    tick: u32,
    tray: Option<tray::Tray>,
}

fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

fn duration_text(secs: f64) -> String {
    let m = (secs / 60.0).round() as u64;
    if m >= 60 { format!("{}h{:02}", m / 60, m % 60) } else { format!("{m} min") }
}

fn color(i: i64) -> Color {
    let (r, g, b) = PALETTE[(i.max(0) as usize) % PALETTE.len()];
    Color::from_rgb_u8(r, g, b)
}

impl Ctx {
    fn ui(&self) -> App {
        self.ui.upgrade().expect("a janela existe enquanto o app roda")
    }

    fn toast(&mut self, text: &str) {
        self.ui().set_toast(text.into());
        self.toast_until = Some(Instant::now() + Duration::from_secs(4));
    }

    fn progress(&self, id: &str) -> String {
        self.db.setting(&progress_key(id)).ok().flatten().unwrap_or_default()
    }

    fn row(&self, m: &Meeting, snippet: Option<&str>) -> MeetingRow {
        let (status, busy, failed) = match m.stage {
            Stage::Recording => ("gravando".to_string(), true, false),
            Stage::Recorded => ("na fila".to_string(), false, false),
            Stage::Processing => {
                let p = self.progress(&m.id);
                (if p.is_empty() { "processando".into() } else { p }, true, false)
            }
            Stage::Done => (String::new(), false, false),
            Stage::Failed => ("falhou".to_string(), false, true),
        };
        let meta = match snippet {
            Some(s) => s.to_string(),
            None => format!("{} · {}", br_datetime(&m.started_at), duration_text(m.duration_secs)),
        };
        MeetingRow { id: (&m.id).into(), title: (&m.title).into(), meta: meta.into(), status: status.into(), busy, failed }
    }

    fn refresh_list(&mut self) {
        let q = self.ui().get_query().to_string();
        let rows: Vec<MeetingRow> = if q.trim().is_empty() {
            self.db.meetings().unwrap_or_default().iter().map(|m| self.row(m, None)).collect()
        } else {
            self.db
                .search(q.trim(), 50)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|h| self.db.meeting(&h.meeting_id).ok().map(|m| self.row(&m, Some(&h.snippet.replace('\n', " ")))))
                .collect()
        };
        self.ui().set_meetings(ModelRc::new(VecModel::from(rows)));
    }

    fn select(&mut self, id: &str) {
        self.ui().set_view("reuniao".into());
        self.ui().set_selected_id(id.into());
        self.ui().set_rename_cluster(-1);
        self.load_current(id);
    }

    fn load_current(&mut self, id: &str) {
        let ui = self.ui();
        let Ok(m) = self.db.meeting(id) else {
            self.cur = None;
            ui.set_selected_id("".into());
            return;
        };
        let segments = self.db.segments(id).unwrap_or_default();
        let speakers = self.db.speakers(id).unwrap_or_default();
        let minutes = self.db.minutes(id).ok().flatten();
        if self.player_id.as_deref() != Some(id) {
            self.player.unload();
            self.player_id = None;
        }
        ui.set_d_title((&m.title).into());
        ui.set_d_meta(format!("{} · {}{}", br_datetime(&m.started_at), duration_text(m.duration_secs), if m.mode == "presencial" { " · presencial" } else { "" }).into());
        let (status, busy, failed) = match m.stage {
            Stage::Done => (m.error.clone().map(|e| format!("Pronta, mas a ata falhou: {e}")).unwrap_or_default(), false, m.error.is_some()),
            Stage::Failed => (format!("Falhou: {}", m.error.clone().unwrap_or_default()), false, true),
            Stage::Processing => (format!("Processando… {}", self.progress(id)), true, false),
            Stage::Recording => ("Gravando…".into(), true, false),
            Stage::Recorded => ("Gravada, ainda não processada.".into(), false, false),
        };
        ui.set_d_status(status.into());
        ui.set_d_busy(busy);
        ui.set_d_failed(failed);
        let has_audio = m.audio_path.as_deref().map(|p| std::path::Path::new(p).exists()).unwrap_or(false) && !busy;
        ui.set_has_audio(has_audio);
        ui.set_dur(m.duration_secs as f32);
        ui.set_dur_text(fmt_time(m.duration_secs).into());
        ui.set_pos(0.0);
        ui.set_pos_text("00:00".into());
        ui.set_playing(false);

        let name = |c: Option<i64>| {
            c.and_then(|c| speakers.iter().find(|s| s.cluster == c)).map(Speaker::label).unwrap_or_else(|| "—".into())
        };
        self.lines.set_vec(
            segments
                .iter()
                .map(|s| Line {
                    start: s.start as f32,
                    time: fmt_time(s.start).into(),
                    speaker: name(s.cluster).into(),
                    text: (&s.text).into(),
                    color: color(s.cluster.unwrap_or(0)),
                    active: false,
                })
                .collect::<Vec<_>>(),
        );
        self.active_line = None;
        ui.set_lines(ModelRc::from(self.lines.clone()));
        ui.set_speakers(ModelRc::new(VecModel::from(
            speakers
                .iter()
                .map(|s| SpeakerRow {
                    cluster: s.cluster as i32,
                    name: s.label().into(),
                    detail: format!("{} de fala{}", fmt_time(s.seconds), if s.voice_id.is_some() { " · voz salva" } else { "" }).into(),
                    color: color(s.cluster),
                })
                .collect::<Vec<_>>(),
        )));
        match &minutes {
            Some(mi) => {
                ui.set_d_summary((&mi.summary).into());
                ui.set_d_decisions(strings(&mi.decisions));
                ui.set_d_open(strings(&mi.open_points));
                ui.set_d_tasks(ModelRc::new(VecModel::from(
                    mi.tasks
                        .iter()
                        .map(|t| TaskRow {
                            text: (&t.text).into(),
                            who: [t.assignee.clone(), t.due.as_ref().map(|d| format!("até {d}"))].into_iter().flatten().collect::<Vec<_>>().join(" · ").into(),
                            done: t.done,
                        })
                        .collect::<Vec<_>>(),
                )));
            }
            None => {
                ui.set_d_summary("".into());
                ui.set_d_decisions(strings(&[]));
                ui.set_d_open(strings(&[]));
                ui.set_d_tasks(ModelRc::new(VecModel::from(Vec::<TaskRow>::new())));
            }
        }
        self.cur = Some(Current { meeting: m, segments, speakers, minutes });
    }

    fn start_process(&mut self, id: &str) {
        if secrets::groq_key().ok().flatten().is_none() {
            self.toast("Falta a chave da Groq: coloque em Ajustes.");
            return;
        }
        if self.children.contains_key(id) {
            return;
        }
        let spawned = std::env::current_exe().map_err(anyhow::Error::from).and_then(|exe| Command::new(exe).args(["process", id]).spawn().map_err(Into::into));
        match spawned {
            Ok(child) => {
                let _ = self.db.set_stage(id, Stage::Processing, None);
                self.children.insert(id.into(), child);
            }
            Err(e) => self.toast(&format!("Não consegui processar: {e}")),
        }
        self.refresh_list();
        if self.cur.as_ref().map(|c| c.meeting.id.as_str()) == Some(id) {
            self.load_current(id);
        }
    }

    fn tray_recording(&self, on: bool) {
        if let Some(t) = &self.tray {
            t.set_recording(on);
        }
    }

    fn open_record(&mut self) {
        let ui = self.ui();
        ui.set_rec_error("".into());
        ui.set_rec_dialog(true);
    }

    fn start_record(&mut self) {
        let ui = self.ui();
        let online = ui.get_rec_online();
        let people = ui.get_rec_people().trim().parse::<u32>().ok().filter(|n| *n >= 1);
        let m = match pipeline::new_meeting(&self.db, if online { "online" } else { "presencial" }, None, people) {
            Ok(m) => m,
            Err(e) => return ui.set_rec_error(format!("{e:#}").into()),
        };
        let out = std::path::PathBuf::from(m.audio_path.clone().unwrap_or_default());
        match Recorder::start(RecorderConfig { mode: if online { Mode::Online } else { Mode::InPerson }, out, mic_device: None, system_device: None }) {
            Ok(rec) => {
                self.rec = Some(Active { rec, id: m.id.clone() });
                ui.set_rec_dialog(false);
                ui.set_recording(true);
                self.tray_recording(true);
                ui.set_rec_online(online);
                self.refresh_list();
            }
            Err(e) => {
                let _ = self.db.delete_meeting(&m.id);
                let _ = std::fs::remove_dir_all(meeting_dir(&m.id));
                ui.set_rec_error(format!("{e:#}").into());
            }
        }
    }

    fn stop_record(&mut self) {
        let Some(a) = self.rec.take() else { return };
        self.ui().set_recording(false);
        self.tray_recording(false);
        match a.rec.stop() {
            Ok(st) => {
                let path = meeting_dir(&a.id).join("audio.opus");
                let _ = self.db.finish_recording(&a.id, st.seconds, &path.to_string_lossy());
                self.toast(&format!("Gravação salva ({}).", duration_text(st.seconds)));
                self.start_process(&a.id);
                self.select(&a.id);
            }
            Err(e) => {
                let _ = self.db.set_stage(&a.id, Stage::Failed, Some(&format!("{e:#}")));
                self.toast(&format!("A gravação falhou: {e:#}"));
            }
        }
        self.refresh_list();
    }

    fn toggle_play(&mut self) {
        let Some(cur) = &self.cur else { return };
        let id = cur.meeting.id.clone();
        let Some(audio) = cur.meeting.audio_path.clone() else { return };
        if self.player_id.as_deref() != Some(&id) {
            let raw = data_dir().join("tmp");
            let _ = std::fs::create_dir_all(&raw);
            self.ui().set_d_status("Preparando o áudio…".into());
            if let Err(e) = self.player.load(std::path::Path::new(&audio), raw.join(format!("play-{}.raw", std::process::id()))) {
                return self.toast(&format!("Não consegui abrir o áudio: {e:#}"));
            }
            self.ui().set_d_status("".into());
            self.player_id = Some(id);
        }
        if self.player.playing() {
            self.player.pause();
        } else {
            let from = self.player.position();
            if let Err(e) = self.player.play_from(from) {
                self.toast(&format!("{e:#}"));
            }
        }
    }

    fn seek(&mut self, secs: f64) {
        if self.player_id.is_none() {
            self.toggle_play();
            self.player.pause();
        }
        if let Err(e) = self.player.seek(secs) {
            self.toast(&format!("{e:#}"));
        }
    }

    fn seek_line(&mut self, secs: f64) {
        self.seek(secs);
        if !self.player.playing() {
            self.toggle_play();
        }
    }

    fn voices_view(&mut self) {
        let rows: Vec<VoiceRow> = self
            .voices
            .list()
            .unwrap_or_default()
            .into_iter()
            .map(|v| VoiceRow { id: v.id as i32, name: (&v.name).into(), detail: format!("{} de fala{}", fmt_time(v.seconds as f64), if v.is_owner { " · você" } else { "" }).into() })
            .collect();
        let ui = self.ui();
        ui.set_voices(ModelRc::new(VecModel::from(rows)));
        ui.set_voice_edit_id(-1);
        ui.set_voice_merge_from(-1);
    }

    fn settings_view(&mut self) {
        let ui = self.ui();
        ui.set_key_state(match secrets::groq_key() {
            Ok(Some(k)) => format!("Chave salva: {}.", secrets::mask(&k)),
            Ok(None) => "Nenhuma chave salva ainda.".into(),
            Err(e) => format!("Cofre indisponível: {e}"),
        }.into());
        ui.set_cfg_autostart(sys::autostart_enabled());
        ui.set_cfg_hotkey(match &self.tray {
            Some(t) if t.hotkey_ok => format!("Atalho global: {} (começa ou para a gravação). Fechar a janela mantém o app na bandeja.", tray::HOTKEY_TEXT),
            Some(_) => "Atalho global indisponível neste sistema. Fechar a janela mantém o app na bandeja.".into(),
            None => "Sem bandeja neste sistema: fechar a janela encerra o app.".into(),
        }.into());
        ui.set_cfg_dark(self.db.setting("dark").ok().flatten().as_deref() != Some("0"));
        ui.set_cfg_threshold(self.db.setting("voice_threshold").ok().flatten().unwrap_or_else(|| "0.50".into()).into());
    }

    fn start_enroll(&mut self) {
        if self.enroll.is_some() || self.rec.is_some() {
            return;
        }
        let path = data_dir().join("tmp").join(format!("cadastro-{}.opus", std::process::id()));
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        match Recorder::start(RecorderConfig { mode: Mode::InPerson, out: path.clone(), mic_device: None, system_device: None }) {
            Ok(rec) => self.enroll = Some(Enroll { rec, path, started: Instant::now() }),
            Err(e) => self.ui().set_enroll_state(format!("{e:#}").into()),
        }
    }

    fn finish_enroll(&mut self) {
        let Some(e) = self.enroll.take() else { return };
        self.ui().set_enroll_state("Analisando a sua voz…".into());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let r = (|| -> Result<()> {
                e.rec.stop()?;
                if !models::missing(&models_dir()).is_empty() {
                    models::ensure(&models_dir(), |_, _, _| {})?;
                }
                let pcm = load_16k(&e.path, Channel::Mix)?;
                let _ = std::fs::remove_file(&e.path);
                let (emb, secs) = enroll_embedding(&SherpaPaths::locate()?, &pcm)?;
                if secs < 8.0 {
                    return Err(anyhow!("só deu para ouvir {secs:.0} s de fala: tente de novo, falando mais perto do microfone"));
                }
                VoiceStore::open(&db_path())?.add("Minha voz", &emb, secs, true)?;
                Ok(())
            })();
            let _ = tx.send(Msg::Enrolled(r.map_err(|e| format!("{e:#}"))));
        });
    }

    fn poll(&mut self) {
        self.tick += 1;
        let ui = self.ui();
        if self.toast_until.map(|t| Instant::now() > t).unwrap_or(false) {
            ui.set_toast("".into());
            self.toast_until = None;
        }
        for cmd in self.tray.as_ref().map(|t| t.poll()).unwrap_or_default() {
            match cmd {
                tray::Cmd::Show => {
                    let _ = ui.show();
                }
                tray::Cmd::ToggleRecord if self.rec.is_some() => self.stop_record(),
                tray::Cmd::ToggleRecord => {
                    let _ = ui.show();
                    self.open_record();
                }
                tray::Cmd::Quit => {
                    let _ = slint::quit_event_loop();
                }
            }
        }
        if let Some(a) = &self.rec {
            ui.set_rec_time(fmt_time(a.rec.elapsed().as_secs_f64()).into());
            let l = a.rec.levels();
            ui.set_rec_level_mic((l.mic.load(std::sync::atomic::Ordering::Relaxed) as f32 / 1000.0).min(1.0));
            ui.set_rec_level_sys((l.system.load(std::sync::atomic::Ordering::Relaxed) as f32 / 1000.0).min(1.0));
        }
        if let Some(e) = &self.enroll {
            let left = ENROLL_SECS.saturating_sub(e.started.elapsed().as_secs());
            ui.set_enroll_state(format!("Gravando… fale normalmente por mais {left} s").into());
            if left == 0 {
                self.finish_enroll();
            }
        }
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Imported(Ok(m)) => {
                    self.refresh_list();
                    self.start_process(&m.id);
                    self.select(&m.id);
                }
                Msg::Imported(Err(e)) => self.toast(&format!("Importação falhou: {e}")),
                Msg::Enrolled(r) => {
                    ui.set_enroll_state(match r {
                        Ok(()) => "Voz salva como “Minha voz”. Renomeie abaixo.".into(),
                        Err(e) => e,
                    }.into());
                    self.voices_view();
                }
            }
        }
        // processamento em outro processo terminou?
        let done: Vec<String> = self.children.iter_mut().filter_map(|(id, c)| matches!(c.try_wait(), Ok(Some(_)) | Err(_)).then(|| id.clone())).collect();
        for id in &done {
            self.children.remove(id);
        }
        let selected = self.cur.as_ref().map(|c| c.meeting.id.clone());
        if !done.is_empty() || (self.tick % 4 == 0 && !self.children.is_empty()) {
            self.refresh_list();
            if let Some(id) = selected.filter(|id| done.contains(id) || self.children.contains_key(id)) {
                let tab_before = ui.get_tab();
                self.load_current(&id);
                ui.set_tab(tab_before);
            }
            for id in &done {
                if let Ok(m) = self.db.meeting(id) {
                    match (m.stage, m.error) {
                        (Stage::Done, None) => self.toast(&format!("Pronta: {}", m.title)),
                        (_, Some(e)) => self.toast(&format!("Problema em “{}”: {e}", m.title)),
                        _ => {}
                    }
                }
            }
        }
        // player
        if self.player_id.is_some() && self.cur.as_ref().map(|c| Some(&c.meeting.id) == self.player_id.as_ref()).unwrap_or(false) {
            let playing = self.player.playing();
            let pos = self.player.position();
            ui.set_playing(playing);
            ui.set_pos(pos as f32);
            ui.set_pos_text(fmt_time(pos).into());
            ui.set_dur(self.player.duration as f32);
            let idx = self.cur.as_ref().and_then(|c| c.segments.iter().rposition(|s| s.start <= pos + 0.05));
            let idx = if playing || pos > 0.0 { idx } else { None };
            if idx != self.active_line {
                for (i, on) in [(self.active_line, false), (idx, true)] {
                    if let Some(i) = i {
                        if let Some(mut l) = self.lines.row_data(i) {
                            l.active = on;
                            self.lines.set_row_data(i, l);
                        }
                    }
                }
                self.active_line = idx;
            }
        }
    }

    fn export_md(&mut self) {
        let Some(c) = &self.cur else { return };
        let md = export::markdown(&Full { meeting: &c.meeting, speakers: &c.speakers, segments: &c.segments, minutes: c.minutes.as_ref() });
        let path = meeting_dir(&c.meeting.id).join("reuniao.md");
        let r = std::fs::write(&path, md).map_err(anyhow::Error::from).and_then(|_| sys::copy_text(&path.to_string_lossy()));
        match r {
            Ok(()) => self.toast(&format!("Markdown salvo; o caminho está na área de transferência ({}).", path.display())),
            Err(e) => self.toast(&format!("Não consegui exportar: {e:#}")),
        }
    }

    fn send_wiki(&mut self) {
        let Some(c) = &self.cur else { return };
        let f = Full { meeting: &c.meeting, speakers: &c.speakers, segments: &c.segments, minutes: c.minutes.as_ref() };
        let msg = match export::send_to_wiki(&f, &export::wiki_dir()) {
            Ok((p, None)) => format!("Enviado à wiki: {}", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            Ok((_, Some(w))) => w,
            Err(e) => format!("Wiki: {e:#}"),
        };
        self.toast(&msg);
    }
}

fn set_dark(ui: &App, dark: bool) {
    ui.global::<Theme>().set_dark(dark);
    ui.global::<Palette>().set_color_scheme(if dark { slint::language::ColorScheme::Dark } else { slint::language::ColorScheme::Light });
}

fn strings(v: &[String]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(v.iter().map(|s| SharedString::from(s.as_str())).collect::<Vec<_>>()))
}

fn run_ui() -> Result<()> {
    std::fs::create_dir_all(data_dir())?;
    let ui = App::new()?;
    let (tx, rx) = channel();
    let ctx = Rc::new(RefCell::new(Ctx {
        ui: ui.as_weak(),
        db: Db::open(&db_path())?,
        voices: VoiceStore::open(&db_path())?,
        rec: None,
        enroll: None,
        player: player::Player::new(),
        player_id: None,
        children: HashMap::new(),
        cur: None,
        lines: Rc::new(VecModel::default()),
        active_line: None,
        toast_until: None,
        tx,
        rx,
        tick: 0,
        tray: tray::Tray::new(),
    }));

    ui.set_version(if STAMP.is_empty() { format!("v{VERSION}") } else { format!("v{VERSION} · {STAMP}") }.into());
    ui.set_lgpd_text(LGPD.into());
    {
        let mut c = ctx.borrow_mut();
        // reunião que ficou "gravando"/"processando" por queda do app: volta para a fila
        for m in c.db.meetings()? {
            match m.stage {
                Stage::Processing => c.db.set_stage(&m.id, Stage::Recorded, None)?,
                Stage::Recording => {
                    let ok = m.audio_path.as_deref().map(|p| std::path::Path::new(p).metadata().map(|f| f.len() > 0).unwrap_or(false)).unwrap_or(false);
                    c.db.set_stage(&m.id, if ok { Stage::Recorded } else { Stage::Failed }, if ok { None } else { Some("a gravação foi interrompida") })?;
                }
                _ => {}
            }
        }
        let dark = c.db.setting("dark")?.as_deref() != Some("0");
        set_dark(&ui, dark);
        c.refresh_list();
        c.settings_view();
    }

    macro_rules! on {
        ($setter:ident, |$c:ident $(, $a:ident)*| $body:expr) => {{
            let ctx = ctx.clone();
            ui.$setter(move |$($a),*| {
                #[allow(unused_mut)]
                let mut $c = ctx.borrow_mut();
                $body
            });
        }};
    }
    on!(on_select, |c, id| c.select(&id));
    on!(on_search, |c, _q| c.refresh_list());
    on!(on_open_record, |c| c.open_record());
    on!(on_cancel_record, |c| c.ui().set_rec_dialog(false));
    on!(on_start_record, |c| c.start_record());
    on!(on_stop_record, |c| c.stop_record());
    on!(on_set_view, |c, v| {
        c.ui().set_view(v.clone());
        match v.as_str() {
            "vozes" => c.voices_view(),
            "config" => c.settings_view(),
            _ => {}
        }
    });
    on!(on_toggle_play, |c| c.toggle_play());
    on!(on_seek, |c, v| c.seek(v as f64));
    on!(on_seek_line, |c, v| c.seek_line(v as f64));
    on!(on_begin_rename, |c, cl, name| {
        let ui = c.ui();
        ui.set_rename_text(if name.starts_with("Falante ") { "".into() } else { name });
        ui.set_rename_cluster(cl);
    });
    on!(on_cancel_rename, |c| c.ui().set_rename_cluster(-1));
    on!(on_commit_rename, |c, cl, name, remember| {
        let (name, id) = (name.trim().to_string(), c.cur.as_ref().map(|x| x.meeting.id.clone()));
        if let (false, Some(id)) = (name.is_empty(), id) {
            match pipeline::name_speaker(&c.db, &c.voices, &id, cl as i64, &name, remember) {
                Ok(()) => {
                    c.ui().set_rename_cluster(-1);
                    let tab = c.ui().get_tab();
                    c.load_current(&id);
                    c.ui().set_tab(tab);
                }
                Err(e) => c.toast(&format!("{e:#}")),
            }
        }
    });
    on!(on_export_md, |c| c.export_md());
    on!(on_send_wiki, |c| c.send_wiki());
    on!(on_reprocess, |c| {
        if let Some(id) = c.cur.as_ref().map(|x| x.meeting.id.clone()) {
            c.start_process(&id);
        }
    });
    on!(on_toggle_task, |c, i| {
        let Some(id) = c.cur.as_ref().map(|x| x.meeting.id.clone()) else { return };
        if let Some(mi) = c.cur.as_mut().and_then(|x| x.minutes.as_mut()) {
            if let Some(t) = mi.tasks.get_mut(i as usize) {
                t.done = !t.done;
            }
            let m = mi.clone();
            if let Err(e) = c.db.save_minutes(&id, &m) {
                return c.toast(&format!("{e:#}"));
            }
            let tab = c.ui().get_tab();
            c.load_current(&id);
            c.ui().set_tab(tab);
        }
    });
    on!(on_ask_delete, |c| c.ui().set_confirm_delete(true));
    on!(on_delete_meeting, |c, yes| {
        c.ui().set_confirm_delete(false);
        if let (true, Some(id)) = (yes, c.cur.as_ref().map(|x| x.meeting.id.clone())) {
            c.player.unload();
            c.player_id = None;
            if let Some(mut ch) = c.children.remove(&id) {
                let _ = ch.kill();
                let _ = ch.wait();
            }
            let _ = c.db.delete_meeting(&id);
            let _ = std::fs::remove_dir_all(meeting_dir(&id));
            c.cur = None;
            c.ui().set_selected_id("".into());
            c.refresh_list();
        }
    });
    on!(on_copy_lgpd, |c| match sys::copy_text(LGPD) {
        Ok(()) => c.toast("Aviso copiado."),
        Err(e) => c.toast(&format!("{e:#}")),
    });
    on!(on_save_key, |c| {
        let k = c.ui().get_key_input().trim().to_string();
        match secrets::set_groq_key(&k) {
            Ok(()) => {
                c.ui().set_key_input("".into());
                c.settings_view();
                c.toast("Chave salva no cofre do sistema.");
            }
            Err(e) => c.toast(&format!("{e:#}")),
        }
    });
    on!(on_clear_key, |c| {
        let _ = secrets::clear_groq_key();
        c.settings_view();
    });
    on!(on_set_dark, |c, d| {
        let _ = c.db.set_setting("dark", if d { "1" } else { "0" });
        set_dark(&c.ui(), d);
    });
    on!(on_set_autostart, |c, on| {
        if let Err(e) = sys::set_autostart(on) {
            c.toast(&format!("{e:#}"));
        }
        c.settings_view();
    });
    on!(on_set_threshold, |c, t| match t.trim().replace(',', ".").parse::<f32>() {
        Ok(v) if (0.3..=0.8).contains(&v) => {
            let _ = c.db.set_setting("voice_threshold", &format!("{v:.2}"));
            c.toast("Limiar salvo.");
        }
        _ => c.toast("Use um valor entre 0,30 e 0,80."),
    });
    on!(on_voice_begin, |c, id, name| {
        let ui = c.ui();
        ui.set_voice_edit(name);
        ui.set_voice_edit_id(id);
    });
    on!(on_voice_commit, |c, id, name| {
        if !name.trim().is_empty() {
            let _ = c.voices.rename(id as i64, name.trim());
        }
        c.voices_view();
    });
    on!(on_voice_delete, |c, id| {
        let _ = c.voices.delete(id as i64);
        c.voices_view();
    });
    on!(on_voice_merge, |c, from, to| match (from, to) {
        (-1, -1) => c.ui().set_voice_merge_from(-1),
        (f, -2) => c.ui().set_voice_merge_from(f),
        (f, t) => {
            match c.voices.merge(t as i64, f as i64) {
                Ok(()) => c.toast("Vozes juntadas."),
                Err(e) => c.toast(&format!("{e:#}")),
            }
            c.voices_view();
        }
    });
    on!(on_enroll_owner, |c| c.start_enroll());
    on!(on_import_file, |c| match sys::pick_audio() {
        Ok(Some(path)) => {
            let tx = c.tx.clone();
            c.toast("Importando… (pode levar um pouco)");
            std::thread::spawn(move || {
                let r = Db::open(&db_path()).and_then(|db| pipeline::import(&db, &path, "online", None, None));
                let _ = tx.send(Msg::Imported(r.map_err(|e| format!("{e:#}"))));
            });
        }
        Ok(None) => {}
        Err(e) => c.toast(&format!("{e:#}")),
    });

    let timer = slint::Timer::default();
    {
        let ctx = ctx.clone();
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(250), move || ctx.borrow_mut().poll());
    }
    // com bandeja, fechar a janela só a esconde (a gravação continua); sem bandeja, encerra
    let has_tray = ctx.borrow().tray.is_some();
    ui.window().on_close_requested(move || {
        if has_tray { slint::CloseRequestResponse::HideWindow } else { let _ = slint::quit_event_loop(); slint::CloseRequestResponse::HideWindow }
    });
    if has_tray && std::env::args().any(|a| a == "--minimizado") {
        // inicia só na bandeja
    } else {
        ui.show()?;
    }
    if has_tray && !ctx.borrow().tray.as_ref().map(|t| t.hotkey_ok).unwrap_or(false) {
        ctx.borrow_mut().toast("Atalho global indisponível neste sistema; use a bandeja.");
    }
    slint::run_event_loop_until_quit()?;
    // fechou durante a gravação: salva o que já foi gravado
    let mut c = ctx.borrow_mut();
    c.stop_record();
    for (_, mut ch) in c.children.drain() {
        let _ = ch.kill();
    }
    Ok(())
}
