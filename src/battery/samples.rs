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

