//! Filter ADC failures before estimating the battery's terminal voltage.

const SAMPLES: usize = 20;

pub fn millivolts(mut read: impl FnMut() -> Option<i32>) -> Option<u32> {
    let mut samples = [0_i32; SAMPLES];
    let mut count = 0;
    for _ in 0..SAMPLES {
        if let Some(raw) = read() {
            samples[count] = raw;
            count += 1;
        }
    }
    if count < SAMPLES / 2 {
        return None;
    }
    samples[..count].sort_unstable();
    Some((samples[count / 2] as u32 * 3300 / 4095) * 2 + 29)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_reads_do_not_enter_the_battery_median() {
        // Review reproduced 29 mV instead of 3815 mV when the first eight
        // reads failed: zero-filled slots were counted as valid samples.
        let mut readings = [None; 8].into_iter().chain([Some(2350); 12]);
        assert_eq!(millivolts(|| readings.next().flatten()), Some(3815));

        let mut readings = [None, Some(2350)].into_iter().cycle();
        assert_eq!(millivolts(|| readings.next().flatten()), Some(3815));
    }

    #[test]
    fn fewer_than_half_successful_reads_leave_voltage_unknown() {
        let mut readings = [Some(2350); 9].into_iter().chain([None; 11]);
        assert_eq!(millivolts(|| readings.next().flatten()), None);
    }

    #[test]
    fn median_rejects_successful_but_noisy_readings() {
        let mut readings = [Some(0); 4]
            .into_iter()
            .chain([Some(2350); 12])
            .chain([Some(4095); 4]);
        assert_eq!(millivolts(|| readings.next().flatten()), Some(3815));
    }
}
