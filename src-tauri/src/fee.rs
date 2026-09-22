//! Fee estimation.

pub fn rate_for(usd: Option<f64>) -> f64 {
    match usd {
        Some(u) if u.is_finite() && u < 5.0 => 0.03,
        _ => 0.01,
    }
}

pub fn fee_minor(amount_minor: u128, usd: Option<f64>) -> u128 {
    let fee = (amount_minor as f64) * rate_for(usd);
    if fee.is_finite() && fee >= 1.0 {
        fee.floor() as u128
    } else {
        0
    }
}

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

        assert_eq!(fee_minor(1_000_000, Some(50.0)), 10_000);

        assert_eq!(fee_minor(1_000_000, Some(2.0)), 30_000);
    }

    #[test]
    fn a_tiny_amount_owes_no_fee() {

        assert_eq!(fee_minor(10, Some(0.01)), 0);
        assert_eq!(fee_minor(0, Some(100.0)), 0);
    }

    #[test]
    fn sending_to_the_fee_address_is_free() {
        assert_eq!(resolve(1_000_000, Some(2.0), "creatorAddr", "creatorAddr"), 0);

        assert_eq!(resolve(1_000_000, Some(2.0), " creatorAddr ", "creatorAddr"), 0);
    }

    #[test]
    fn a_normal_send_carries_the_fee() {
        assert_eq!(resolve(1_000_000, Some(2.0), "someoneElse", "creatorAddr"), 30_000);
    }
}
