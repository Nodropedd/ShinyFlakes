//! The creator fee added to a send.
//!
//! A small cut goes to the donation addresses: 3% on sends worth under five
//! dollars, 1% at or above. It is always an extra output on top of the
//! amount, shown as its own line before anything is signed, never hidden.
//!
//! Two things switch it off: a send already going to the fee address for that
//! asset (no point paying yourself), and a fee that rounds to nothing.

/// The rate for a send, from its value in dollars.
///
/// When the value is unknown, because no price was available, the gentler 1%
/// is used rather than the higher tier, so an unpriced send is never
/// overcharged.
pub fn rate_for(usd: Option<f64>) -> f64 {
    match usd {
        Some(u) if u.is_finite() && u < 5.0 => 0.03,
        _ => 0.01,
    }
}

/// The fee in the asset's smallest unit.
///
/// Floored, so rounding never charges a fraction of a unit more than the rate.
pub fn fee_minor(amount_minor: u128, usd: Option<f64>) -> u128 {
    let fee = (amount_minor as f64) * rate_for(usd);
    if fee.is_finite() && fee >= 1.0 {
        fee.floor() as u128
    } else {
        0
    }
}

/// The fee for a send, or zero when it should not apply: paying the fee
/// address itself, or a fee too small to be worth an output.
pub fn resolve(amount_minor: u128, usd: Option<f64>, to: &str, fee_address: &str) -> u128 {
    if to.trim() == fee_address.trim() {
        return 0;
    }
    fee_minor(amount_minor, usd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_sends_pay_three_percent() {
        assert_eq!(rate_for(Some(4.99)), 0.03);
        assert_eq!(rate_for(Some(0.10)), 0.03);
    }

    #[test]
    fn larger_sends_pay_one_percent() {
        assert_eq!(rate_for(Some(5.0)), 0.01);
        assert_eq!(rate_for(Some(1000.0)), 0.01);
    }

    #[test]
    fn an_unknown_value_takes_the_gentler_rate() {
        assert_eq!(rate_for(None), 0.01);
        assert_eq!(rate_for(f64::NAN.into()), 0.01);
    }

    #[test]
    fn the_fee_is_a_floored_cut_of_the_amount() {
        // 1% of 1_000_000 is 10_000.
        assert_eq!(fee_minor(1_000_000, Some(50.0)), 10_000);
        // 3% of 1_000_000 is 30_000.
        assert_eq!(fee_minor(1_000_000, Some(2.0)), 30_000);
    }

    #[test]
    fn a_tiny_amount_owes_no_fee() {
        // 3% of 10 is 0.3, which floors below one unit.
        assert_eq!(fee_minor(10, Some(0.01)), 0);
        assert_eq!(fee_minor(0, Some(100.0)), 0);
    }

    #[test]
    fn sending_to_the_fee_address_is_free() {
        assert_eq!(resolve(1_000_000, Some(2.0), "creatorAddr", "creatorAddr"), 0);
        // Whitespace differences do not defeat the check.
        assert_eq!(resolve(1_000_000, Some(2.0), " creatorAddr ", "creatorAddr"), 0);
    }

    #[test]
    fn a_normal_send_carries_the_fee() {
        assert_eq!(resolve(1_000_000, Some(2.0), "someoneElse", "creatorAddr"), 30_000);
    }
}
