//! Original cubic timing presets. These are editable approximations of familiar easing families,
//! not imported presets or artwork. x is time, y is normalized change in the parameter value.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EasingCurve(pub [f64; 4]);
impl Default for EasingCurve {
    fn default() -> Self {
        Self([1.0 / 3.0, 0.0, 2.0 / 3.0, 1.0])
    }
}
impl EasingCurve {
    pub fn valid(self) -> bool {
        let [x1, y1, x2, y2] = self.0;
        self.0.iter().all(|v| v.is_finite())
            && (0.0..=1.0).contains(&x1)
            && (0.0..=1.0).contains(&x2)
            && (-2.0..=3.0).contains(&y1)
            && (-2.0..=3.0).contains(&y2)
    }
    pub fn sample(self, x: f64) -> f64 {
        if !x.is_finite() {
            return 0.0;
        }
        let x = x.clamp(0.0, 1.0);
        if !self.valid() || x == 0.0 || x == 1.0 {
            return x;
        }
        let [a, b, c, d] = self.0;
        crate::keyframe::cubic_bezier_y_for_x(a, b, c, d, x)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EasingPreset {
    pub name: String,
    pub curve: EasingCurve,
}
/// Cubic approximations with paired mirrored In/Out handles and a symmetric InOut version.
pub fn presets() -> Vec<EasingPreset> {
    let mut out = vec![EasingPreset { name: "Linear".into(), curve: EasingCurve([0.0, 0.0, 1.0, 1.0]) }];
    for (name, c, both) in [
        ("Ease", [0.42, 0.0, 1.0, 1.0], [0.42, 0.0, 0.58, 1.0]),
        ("Sine", [0.36, 0.0, 0.67, 0.47], [0.37, 0.0, 0.63, 1.0]),
        ("Quad", [1.0 / 3.0, 0.0, 2.0 / 3.0, 1.0 / 3.0], [0.46, 0.0, 0.54, 1.0]),
        ("Cubic", [1.0 / 3.0, 0.0, 2.0 / 3.0, 0.0], [0.65, 0.0, 0.35, 1.0]),
        ("Quart", [0.44, 0.0, 0.76, 0.0], [0.77, 0.0, 0.23, 1.0]),
        ("Quint", [0.53, 0.0, 0.81, 0.0], [0.84, 0.0, 0.16, 1.0]),
        ("Expo", [0.7, 0.0, 0.86, 0.0], [0.9, 0.0, 0.1, 1.0]),
        ("Circ", [0.52, 0.0, 1.0, 0.44], [0.85, 0.0, 0.15, 1.0]),
        ("Back", [0.36, -0.55, 0.66, -0.55], [0.68, -0.55, 0.32, 1.55]),
    ] {
        out.push(EasingPreset { name: format!("{name} In"), curve: EasingCurve(c) });
        out.push(EasingPreset { name: format!("{name} Out"), curve: EasingCurve([1.0 - c[2], 1.0 - c[3], 1.0 - c[0], 1.0 - c[1]]) });
        out.push(EasingPreset { name: format!("{name} In Out"), curve: EasingCurve(both) });
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Keyframe, Param, ParamValue};
    use filmcraft_time::Tick;
    #[test]
    fn curves_have_exact_endpoints_and_finite_samples() {
        for p in presets() {
            assert!(p.curve.valid());
            assert_eq!(p.curve.sample(0.0), 0.0);
            assert_eq!(p.curve.sample(1.0), 1.0);
            for i in 0..=100 {
                assert!(p.curve.sample(i as f64 / 100.0).is_finite());
            }
        }
        assert!(!EasingCurve([f64::NAN, 0.0, 1.0, 1.0]).valid());
        assert!(!EasingCurve([-1.0, 0.0, 1.0, 1.0]).valid());
    }
    #[test]
    fn overshoot_scalar_render_and_serialization_agree() {
        let mut p = Param::new(ParamValue::Float(0.0));
        let mut a = Keyframe::new(Tick(0), ParamValue::Float(0.0));
        a.easing = Some(EasingCurve([0.3, 1.6, 0.6, 1.3]));
        p.keyframes = vec![a, Keyframe::new(Tick(100), ParamValue::Float(100.0))];
        assert!(p.f64_at(Tick(65)) > 100.0);
        assert_eq!(p.f64_at(Tick(65)), p.scalar_at(Tick(65)));
        let restored: Param = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p.f64_at(Tick(65)), restored.f64_at(Tick(65)));
        let legacy: Keyframe = serde_json::from_value(serde_json::json!({"time":0,"value":{"Float":0.0},"interp":"Linear"})).unwrap();
        assert!(legacy.easing.is_none());
    }
}
