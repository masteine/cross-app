//! FPS numbers from the Metal performance HUD log.
//!
//! With `MTL_HUD_ENABLED=1 MTL_HUD_LOG_ENABLED=1` the HUD writes, once per
//! second, a line to the console (Apple, "Monitoring your Metal app's graphics
//! performance"):
//!
//! ```text
//! metal-HUD: <first-frame-number>,<graphics-memory>,<process-memory>,<present-interval>,<gpu-time>,...
//! ```
//!
//! with one `<present-interval>,<gpu-time>` pair per frame of that second. The
//! number of pairs on a line is therefore that second's frame rate, which needs
//! no knowledge of the (undocumented) units. Whether the lines reach the
//! program's stderr (and so our logs) or only the unified log is not
//! documented either; `log show --predicate 'eventMessage CONTAINS "metal-HUD:"'`
//! exports them from the latter. Not verified with a real game here.

use serde::Serialize;

const MARKER: &str = "metal-HUD:";

/// One `metal-HUD:` line.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HudSample {
    pub first_frame: u64,
    pub graphics_memory: f64,
    pub process_memory: f64,
    /// `(present interval, GPU time)` per frame, in the HUD's own units.
    pub frames: Vec<(f64, f64)>,
}

/// Parse a line that contains `metal-HUD:` anywhere (log prefixes are fine).
pub fn parse_line(line: &str) -> Option<HudSample> {
    let (_, data) = line.split_once(MARKER)?;
    let mut fields = data.trim().split(',').map(str::trim);
    let first_frame = fields.next()?.parse().ok()?;
    let graphics_memory = fields.next()?.parse().ok()?;
    let process_memory = fields.next()?.parse().ok()?;
    let values: Vec<f64> = fields
        .filter(|f| !f.is_empty())
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    if values.len() % 2 != 0 {
        return None;
    }
    let frames = values.chunks(2).map(|p| (p[0], p[1])).collect();
    Some(HudSample {
        first_frame,
        graphics_memory,
        process_memory,
        frames,
    })
}

/// FPS statistics over a log.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FpsSummary {
    /// Number of `metal-HUD:` lines, i.e. seconds covered.
    pub seconds: usize,
    pub frames: usize,
    pub avg_fps: f64,
    pub min_fps: usize,
    pub max_fps: usize,
    /// Mean present interval and GPU time per frame, in the HUD's units.
    pub mean_present_interval: f64,
    pub mean_gpu_time: f64,
}

/// Summarize every `metal-HUD:` line in `text`; `None` if there are none.
pub fn summarize(text: &str) -> Option<FpsSummary> {
    let samples: Vec<HudSample> = text.lines().filter_map(parse_line).collect();
    if samples.is_empty() {
        return None;
    }
    let per_second: Vec<usize> = samples.iter().map(|s| s.frames.len()).collect();
    let frames: usize = per_second.iter().sum();
    let mean = |pick: fn(&(f64, f64)) -> f64| {
        if frames == 0 {
            return 0.0;
        }
        samples
            .iter()
            .flat_map(|s| s.frames.iter().map(pick))
            .sum::<f64>()
            / frames as f64
    };
    Some(FpsSummary {
        seconds: samples.len(),
        frames,
        avg_fps: frames as f64 / samples.len() as f64,
        min_fps: per_second.iter().copied().min().unwrap_or(0),
        max_fps: per_second.iter().copied().max().unwrap_or(0),
        mean_present_interval: mean(|f| f.0),
        mean_gpu_time: mean(|f| f.1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hud_lines_with_and_without_prefixes() {
        let s = parse_line("metal-HUD: 120,512.5,900.25,16.6,8.1,16.8,8.3").unwrap();
        assert_eq!(s.first_frame, 120);
        assert_eq!(s.graphics_memory, 512.5);
        assert_eq!(s.frames, [(16.6, 8.1), (16.8, 8.3)]);

        let prefixed = "2026-10-15 12:00:01 game[123] metal-HUD: 1,1,1,33.3,10";
        assert_eq!(parse_line(prefixed).unwrap().frames.len(), 1);

        assert_eq!(parse_line("nothing here"), None);
        assert_eq!(parse_line("metal-HUD: x,1,1"), None);
        assert_eq!(parse_line("metal-HUD: 1,1,1,16.6"), None, "odd pair count");
        assert_eq!(parse_line("metal-HUD: 1,1,1").unwrap().frames.len(), 0);
    }

    #[test]
    fn frame_rate_is_pairs_per_line() {
        let log = "\
start
metal-HUD: 1,10,20,16,8,16,8,16,8
noise metal-HUD: 4,10,20,33,9
metal-HUD: 5,10,20,16,7,17,7
";
        let sum = summarize(log).unwrap();
        assert_eq!(sum.seconds, 3);
        assert_eq!(sum.frames, 6);
        assert_eq!(sum.avg_fps, 2.0);
        assert_eq!((sum.min_fps, sum.max_fps), (1, 3));
        assert!((sum.mean_present_interval - 19.0).abs() < 1e-9);
        assert_eq!(summarize("no hud"), None);
    }
}
