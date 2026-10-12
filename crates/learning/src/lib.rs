//! M9 Learning data loop. Everything here is an aggregate over the book: partner insights with
//! small cohorts withheld (R21, `insights`), experience studies for the actuary (`experience`,
//! ADR-012) and the pilot's metrics for its gates (`pilot`). No customer id leaves; optional data
//! is used only for the cover it supports and erased on cancellation (R20, R54).
pub mod experience;
pub mod insights;
pub mod pilot;
