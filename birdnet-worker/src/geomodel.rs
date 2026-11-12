//! The BirdNET+ Geomodel (v3.0.4, from birdnet-team/geomodel, Apache 2.0):
//! latitude, longitude and week in, a 0 to 1 occurrence score for each of its
//! 14,082 species out. Built, run and dropped per query; it loads in about
//! 0.2 s and answers in about 2 ms.

use tract_onnx::prelude::*;

type Plan = std::sync::Arc<TypedRunnableModel>;

pub struct Geomodel {
    plan: Plan,
}

impl Geomodel {
    pub fn load(bytes: &[u8]) -> TractResult<Geomodel> {
        let plan = tract_onnx::onnx()
            .with_ignore_value_info(true)
            .with_ignore_output_shapes(true)
            .model_for_read(&mut std::io::Cursor::new(bytes))?
            .with_input_fact(0, f32::fact([1, 3]).into())?
            .into_optimized()?
            .into_runnable()?;
        Ok(Geomodel { plan })
    }

    fn week_scores(&self, lat: f32, lon: f32, week: f32) -> TractResult<Vec<f32>> {
        let input = Tensor::from_shape(&[1, 3], &[lat, lon, week])?;
        let out = self.plan.run(tvec!(input.into()))?;
        Ok(out[0]
            .to_plain_array_view::<f32>()?
            .iter()
            .copied()
            .collect())
    }

    /// Scores for `week` 1 to 48, or for week 0 the highest of each species
    /// over all 48 weeks (upstream's "yearly" list).
    pub fn scores(&self, lat: f32, lon: f32, week: u32) -> TractResult<Vec<f32>> {
        if (1..=48).contains(&week) {
            return self.week_scores(lat, lon, week as f32);
        }
        let mut best = self.week_scores(lat, lon, 1.0)?;
        for w in 2..=48 {
            for (b, s) in best.iter_mut().zip(self.week_scores(lat, lon, w as f32)?) {
                *b = b.max(s);
            }
        }
        Ok(best)
    }
}
