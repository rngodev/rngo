use crate::signal::SqlSignal;
pub use crate::signal::sql::SqlSignalBuilder;

pub fn sql_signal() -> SqlSignalBuilder {
    SqlSignal::builder()
}
