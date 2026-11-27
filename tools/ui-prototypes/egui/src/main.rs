use eframe::egui;
struct A { q: String, pos: f32, note: String }
impl eframe::App for A {
    fn ui(&mut self, ui: &mut egui::Ui, _f: &mut eframe::Frame) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.text_edit_singleline(&mut self.q);
            ui.add(egui::Slider::new(&mut self.pos, 0.0..=1.0));
            egui::ScrollArea::vertical().max_height(380.0).show_rows(ui, 24.0, 500, |ui, r| {
                for i in r { ui.label(format!("Falante {} — fala de exemplo número {}", i % 4, i)); }
            });
            ui.text_edit_multiline(&mut self.note);
            let _ = ui.button("Gravar");
        });
    }
}
fn main() -> eframe::Result {
    let o = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 600.0]), ..Default::default() };
    eframe::run_native("proto", o, Box::new(|_| Ok(Box::new(A { q: String::new(), pos: 0.3, note: "nota".into() }))))
}
