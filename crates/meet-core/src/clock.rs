//! Hora de Brasília. O Brasil não tem horário de verão desde 2019, então um deslocamento fixo de -03:00 basta.

use chrono::{DateTime, FixedOffset, Utc};

fn brasilia() -> FixedOffset {
    FixedOffset::west_opt(3 * 3600).expect("offset válido")
}

pub fn now_sp() -> DateTime<FixedOffset> {
    Utc::now().with_timezone(&brasilia())
}

/// `2026-11-27T10:00:00-03:00` → `27/11/2026 10:00`. Texto inválido volta como veio.
pub fn br_datetime(rfc3339: &str) -> String {
    DateTime::parse_from_rfc3339(rfc3339)
        .map(|d| d.with_timezone(&brasilia()).format("%d/%m/%Y %H:%M").to_string())
        .unwrap_or_else(|_| rfc3339.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formata_em_brasilia() {
        assert_eq!(br_datetime("2026-11-27T10:00:00-03:00"), "27/11/2026 10:00");
        assert_eq!(br_datetime("2026-11-27T13:00:00Z"), "27/11/2026 10:00");
        assert_eq!(br_datetime("lixo"), "lixo");
        assert_eq!(now_sp().offset().local_minus_utc(), -3 * 3600);
    }
}
