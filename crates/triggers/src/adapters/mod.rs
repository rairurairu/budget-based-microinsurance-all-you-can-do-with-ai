pub mod cap;
pub mod flight;
pub mod hko;
pub mod organiser;
pub mod rail;
pub mod transit;

use crate::{SourceError, TriggerSource};
use chrono::NaiveDate;
use microinsurance_engine::triggers::SourceReport;

/// Any built adapter, so a worker can poll a list of configured sources. A source's config file
/// names its adapter (`adapter:`, ADR-012): an adapter written for one feed's own format reports
/// under that feed's id; a standard format (CAP) under the id of the source that uses it.
pub enum Adapter {
    Hko(hko::HkoWarnings),
    Transit(transit::TransitDisruptions),
    Rail(rail::RailStatus),
    Flight(flight::FlightStatus),
    Organiser(organiser::OrganiserNotices),
    Cap(cap::CapAlerts),
}

impl TriggerSource for Adapter {
    fn id(&self) -> &str {
        match self {
            Adapter::Hko(a) => a.id(),
            Adapter::Transit(a) => a.id(),
            Adapter::Rail(a) => a.id(),
            Adapter::Flight(a) => a.id(),
            Adapter::Organiser(a) => a.id(),
            Adapter::Cap(a) => a.id(),
        }
    }

    async fn poll(&self) -> Result<Vec<SourceReport>, SourceError> {
        match self {
            Adapter::Hko(a) => a.poll().await,
            Adapter::Transit(a) => a.poll().await,
            Adapter::Rail(a) => a.poll().await,
            Adapter::Flight(a) => a.poll().await,
            Adapter::Organiser(a) => a.poll().await,
            Adapter::Cap(a) => a.poll().await,
        }
    }

    async fn history(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<SourceReport>, SourceError> {
        match self {
            Adapter::Hko(a) => a.history(from, to).await,
            Adapter::Transit(a) => a.history(from, to).await,
            Adapter::Rail(a) => a.history(from, to).await,
            Adapter::Flight(a) => a.history(from, to).await,
            Adapter::Organiser(a) => a.history(from, to).await,
            Adapter::Cap(a) => a.history(from, to).await,
        }
    }
}
