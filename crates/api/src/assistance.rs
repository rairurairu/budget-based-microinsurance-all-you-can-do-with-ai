//! The guarantee-of-payment exchange with the assistance provider (R46, ADR-010). The engine
//! guarantees up to `auto_guarantee_cap` at once; anything above it is sent to the assistance
//! provider to confirm. The API records every request (`GUARANTEES`) with the provider's
//! reference, so a person sees what is waiting; each insurer connects its own provider.
use microinsurance_engine::claims::Claim;
use microinsurance_engine::money::Money;

pub trait AssistanceProvider {
    /// Asks the provider to confirm a guarantee of `to_confirm` for the claim; returns its
    /// reference.
    fn request_guarantee(&self, claim: &Claim, to_confirm: Money) -> Result<String, String>;
}

/// No provider connected: the request is recorded for a person to send on.
pub struct RecordOnly;

impl AssistanceProvider for RecordOnly {
    fn request_guarantee(&self, claim: &Claim, _to_confirm: Money) -> Result<String, String> {
        Ok(format!("recorded/{}", claim.claim_id))
    }
}
