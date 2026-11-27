use iced::widget::{button, column, scrollable, slider, text, text_input, Column};
use iced::Element;
#[derive(Default)]
struct A { q: String, pos: f32, note: String }
#[derive(Clone, Debug)]
enum M { Q(String), P(f32), N(String), Go }
fn update(a: &mut A, m: M) { match m { M::Q(s) => a.q = s, M::P(p) => a.pos = p, M::N(s) => a.note = s, M::Go => {} } }
fn view(a: &A) -> Element<'_, M> {
    let rows = Column::with_children((0..500).map(|i| text(format!("Falante {} — fala de exemplo número {}", i % 4, i)).into()));
    column![text_input("Buscar", &a.q).on_input(M::Q), slider(0.0..=1.0, a.pos, M::P).step(0.01), scrollable(rows).height(380), text_input("nota", &a.note).on_input(M::N), button("Gravar").on_press(M::Go)].spacing(8).padding(12).into()
}
fn main() -> iced::Result { iced::application(A::default, update, view).window_size((900.0, 600.0)).run() }
