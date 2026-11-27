slint::include_modules!();
fn main() {
    let app = App::new().unwrap();
    let rows: Vec<slint::SharedString> = (0..500).map(|i| format!("Falante {} — fala de exemplo número {}", i % 4, i).into()).collect();
    app.set_rows(slint::ModelRc::new(slint::VecModel::from(rows)));
    app.run().unwrap();
}
