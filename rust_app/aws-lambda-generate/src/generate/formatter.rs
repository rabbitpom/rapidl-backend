use ::std::fmt::Write; 
use rand::seq::SliceRandom;

pub const LABEL_KMH: &'static str = r#"\text{kmh}^{-1}"#;
pub const LABEL_KMS: &'static str = r#"\text{kms}^{-1}"#;
pub const LABEL_MH: &'static str = r#"\text{mh}^{-1}"#;
pub const LABEL_MS: &'static str = r#"\text{ms}^{-1}"#;
pub const LABEL_KG: &'static str = r#"\text{kg}"#;
pub const LABEL_G: &'static str = r#"\text{g}"#;
pub const LABEL_KAS: &'static str = r#"\text{kms}^{-2}"#;
pub const LABEL_KAH: &'static str = r#"\text{kmh}^{-2}"#;
pub const LABEL_AS: &'static str = r#"\text{ms}^{-2}"#;
pub const LABEL_AH: &'static str = r#"\text{mh}^{-2}"#;
pub const LABEL_M: &'static str = r#"\text{m}"#;
pub const LABEL_KM: &'static str = r#"\text{km}"#;

pub const LABEL_KMH_RAW: &'static str = "kmh^-1";
pub const LABEL_KMS_RAW: &'static str = "kms^-1";
pub const LABEL_MH_RAW: &'static str = "mh^-1";
pub const LABEL_MS_RAW: &'static str = "ms^-1";
pub const LABEL_KG_RAW: &'static str = "kg";
pub const LABEL_G_RAW: &'static str = "g";
pub const LABEL_KAS_RAW: &'static str = "kms^-2";
pub const LABEL_KAH_RAW: &'static str = "kmh^-2";
pub const LABEL_AS_RAW: &'static str = "ms^-2";
pub const LABEL_AH_RAW: &'static str = "mh^-2";
pub const LABEL_M_RAW: &'static str = r#"m"#;
pub const LABEL_KM_RAW: &'static str = r#"km"#;

pub const LABELLED_SYMBOLS: [&'static str; 10] = [
    r#"\alpha"#,
    r#"\beta"#,
    r#"\gamma"#,
    r#"\delta"#,
    r#"\epsilon"#,
    r#"\zeta"#,
    r#"\eta"#,
    r#"\theta"#,
    r#"\iota"#,
    r#"\kappa"#,
];
pub const LABELLED_IDENTIFIERS_RAW: [&'static str; 25] = [
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", 
    "N", "O", "P", "Q", "R", "S", "U", "V", "W", "X", "Y", "Z"
];
pub const LABELLED_IDENTIFIERS: [&'static str; 25] = [
    r#"\mathbf{A}"#,
    r#"\mathbf{B}"#,
    r#"\mathbf{C}"#,
    r#"\mathbf{D}"#,
    r#"\mathbf{E}"#,
    r#"\mathbf{F}"#,
    r#"\mathbf{G}"#,
    r#"\mathbf{H}"#,
    r#"\mathbf{I}"#,
    r#"\mathbf{J}"#,
    r#"\mathbf{K}"#,
    r#"\mathbf{L}"#,
    r#"\mathbf{M}"#,
    r#"\mathbf{N}"#,
    r#"\mathbf{O}"#,
    r#"\mathbf{P}"#,
    r#"\mathbf{Q}"#,
    r#"\mathbf{R}"#,
    r#"\mathbf{S}"#,
    r#"\mathbf{U}"#,
    r#"\mathbf{V}"#,
    r#"\mathbf{W}"#,
    r#"\mathbf{X}"#,
    r#"\mathbf{Y}"#,
    r#"\mathbf{Z}"#,
];
pub const LABELLED_COMPONENTS: [&'static str; 10] = [
    r#"\hat{\mathbf{i}}"#,
    r#"\hat{\mathbf{j}}"#,
    r#"\hat{\mathbf{k}}"#,
    r#"\hat{\mathbf{l}}"#,
    r#"\hat{\mathbf{m}}"#,
    r#"\hat{\mathbf{n}}"#,
    r#"\hat{\mathbf{w}}"#,
    r#"\hat{\mathbf{p}}"#,
    r#"\hat{\mathbf{q}}"#,
    r#"\hat{\mathbf{r}}"#,
];
pub const LABELLED_COMPONENTS_RAW: [&'static str; 10] = ["i", "j", "k", "l", "m", "n", "w", "p", "q", "r"];
pub const DEFAULT_SIG_FIGURES: usize = 3;

pub fn math_mode<T: ::std::fmt::Display>(inner: T) -> String {
    format!(r#"\({}\)"#, inner)
}

pub fn gcd(mut a: i32, mut b: i32) -> i32 {
    while b != 0 {
        let temp = b;
        b = a % b;
        a = temp;
    }
    a
}

pub fn simplify_square_root(n: i32) -> (i32, i32) {
    if n <= 0 {
        return (0, 0);
    }
    if n == 1 {
        return (1, 1);
    }

    let mut remaining = n;
    let mut mul = 1;
    for i in 2..=n {
        while remaining % (i * i) == 0 {
            mul *= i;
            remaining /= i * i;
        }
    }
    if mul == 1 {
        return (1, n);
    }
    return (mul, remaining);
}

fn f32_significant_figures(float: f32, precision: usize) -> f32 {
    // compute absolute value
    let a = float.abs();

    // if abs value is greater than 1, then precision becomes less than "standard"
    let precision = if a >= 1. {
        // reduce by number of digits, minimum 0
        let n = (1. + a.log10().floor()) as usize;
        if n <= precision {
            precision - n
        } else {
            0
        }
    // if precision is less than 1 (but non-zero), then precision becomes greater than "standard"
    } else if a > 0. {
        // increase number of digits
        let n = -(1. + a.log10().floor()) as usize;
        precision + n
    // special case for 0
    } else {
        0
    };

    // calculate the scaling factor to multiply by and round to nearest integer
    let scaling_factor = 10_f32.powi(precision as i32);
    let scaled_value = (float * scaling_factor).round() / scaling_factor;

    scaled_value
}

fn format_f32_significant_figures(float: f32, precision: usize) -> String {
    let scaled_value = f32_significant_figures(float, precision);
    format!("{0:.1$}", scaled_value, precision)
}

pub fn format_f32_raw(float: f32, precision: Option<usize>) -> String {
    format_f32_significant_figures(float, precision.unwrap_or(DEFAULT_SIG_FIGURES))
}

pub fn format_f32(float: f32, precision: Option<usize>) -> String {
    let raw_format = format_f32_raw(float, precision);
    math_mode(raw_format)
}

pub fn format_i32_raw(int: i32) -> String {
    int.to_string()
}

pub fn format_i32(int: i32) -> String {
    let raw_format = format_i32_raw(int);
    math_mode(raw_format)
}

pub fn format_bool_raw(value: bool) -> &'static str {
    match value {
        true => r#"\unicode{x2714}"#,
        false => r#"\unicode{x2718}"#,
    }
}

pub fn format_bool(value: bool) -> String {
    math_mode(format_bool_raw(value))
}

pub fn format_f32_group_labelled_raw2(values: &[f32]) -> String {
    let mut result = String::new();
    result.push_str(r#"("#);

    for (i, &value) in values.iter().enumerate() {
        if i > 0 {
            if value >= 0.0 {
                write!(&mut result, "+{}{}", format_f32_raw(value, None), LABELLED_COMPONENTS_RAW[i]).expect("format_f32_group_labelled_raw2 failed to write to string");
            } else {
                write!(&mut result, "{}{}", format_f32_raw(value, None), LABELLED_COMPONENTS_RAW[i]).expect("format_f32_group_labelled_raw2 failed to write to string");
            }
        } else {
            write!(&mut result, "{}{}", format_f32_raw(value, None), LABELLED_COMPONENTS_RAW[i]).expect("format_f32_group_labelled_raw2 failed to write to string");
        }
    }

    result.push_str(r#")"#);
    result
}

pub fn format_f32_group_labelled_raw(values: &[f32]) -> String {
    let mut result = String::new();
    result.push_str(r#"\begin{pmatrix} "#);

    for (i, &value) in values.iter().enumerate() {
        if i > 0 {
            if value >= 0.0 {
                write!(&mut result, "+{}{}", format_f32_raw(value, None), LABELLED_COMPONENTS[i]).expect("format_f32_group_labelled_raw failed to write to string");
            } else {
                write!(&mut result, "{}{}", format_f32_raw(value, None), LABELLED_COMPONENTS[i]).expect("format_f32_group_labelled_raw failed to write to string");
            }
        } else {
            write!(&mut result, "{}{}", format_f32_raw(value, None), LABELLED_COMPONENTS[i]).expect("format_f32_group_labelled_raw failed to write to string");
        }
    }

    result.push_str(r#"\end{pmatrix}"#);
    result
}

pub fn format_i32_group_labelled_raw2(values: &[i32]) -> String {
    let mut result = String::new();
    result.push_str(r#"("#);

    for (i, &value) in values.iter().enumerate() {
        if i > 0 {
            if value >= 0 {
                write!(&mut result, "+{}{}", value, LABELLED_COMPONENTS_RAW[i]).expect("format_i32_group_labelled_raw2 failed to write to string");
            } else {
                write!(&mut result, "{}{}", value, LABELLED_COMPONENTS_RAW[i]).expect("format_i32_group_labelled_raw2 failed to write to string");
            }
        } else {
            write!(&mut result, "{}{}", value, LABELLED_COMPONENTS_RAW[i]).expect("format_i32_group_labelled_raw2 failed to write to string");
        }
    }

    result.push_str(r#")"#);
    result
}

pub fn format_i32_group_labelled_raw(values: &[i32]) -> String {
    let mut result = String::new();
    result.push_str(r#"\begin{pmatrix} "#);

    for (i, &value) in values.iter().enumerate() {
        if i > 0 {
            if value >= 0 {
                write!(&mut result, "+{}{}", value, LABELLED_COMPONENTS[i]).expect("format_i32_group_labelled_raw failed to write to string");
            } else {
                write!(&mut result, "{}{}", value, LABELLED_COMPONENTS[i]).expect("format_i32_group_labelled_raw failed to write to string");
            }
        } else {
            write!(&mut result, "{}{}", value, LABELLED_COMPONENTS[i]).expect("format_i32_group_labelled_raw failed to write to string");
        }
    }

    result.push_str(r#"\end{pmatrix}"#);
    result
}

pub fn format_i32_group_labelled(values: &[i32]) -> String {
    math_mode(format_i32_group_labelled_raw(values))
}

pub fn format_i32_group_raw2(values: &[i32]) -> String {
    let mut result = String::new();
    result.push_str(r#"("#);

    for (i, &value) in values.iter().enumerate() {
        if i > 0 {
            if value >= 0 {
                write!(&mut result, "+{}", value).expect("format_i32_group_raw2 failed to write to string");
            } else {
                write!(&mut result, "{}", value).expect("format_i32_group_raw2 failed to write to string");
            }
        } else {
            write!(&mut result, "{}", value).expect("format_i32_group_raw2 failed to write to string");
        }
    }

    result.push_str(r#")"#);
    result
}

pub fn format_i32_group_raw(values: &[i32]) -> String {
    let mut result = String::new();
    result.push_str(r#"\begin{pmatrix} "#);

    for (i, &value) in values.iter().enumerate() {
        if i > 0 {
            if value >= 0 {
                write!(&mut result, "+{}", value).expect("format_i32_group_raw failed to write to string");
            } else {
                write!(&mut result, "{}", value).expect("format_i32_group_raw failed to write to string");
            }
        } else {
            write!(&mut result, "{}", value).expect("format_i32_group_raw failed to write to string");
        }
    }

    result.push_str(r#"\end{pmatrix}"#);
    result
}

pub fn format_i32_group(values: &[i32]) -> String {
    math_mode(format_i32_group_raw(values))
}

pub fn format_i32_vec_raw_2(values: &[i32]) -> String {
    let mut result = String::new();
    result.push('(');
    for (i, &value) in values.iter().enumerate() {
        if i > 0 {
            result.push_str(", ");
        }
        write!(&mut result, "{}", value).expect("format_i32_vec_raw_2 failed to write to string");
    }
    result.push(')');
    result
}

pub fn format_i32_vec_raw(values: &[i32]) -> String {
    let mut result = String::new();
    result.push_str(r#"\begin{pmatrix} "#);
    for (i, &value) in values.iter().enumerate() {
        write!(&mut result, "{}{}", value, {
            if i < values.len() - 1 {
                ",&"
            } else {
                ""
            }
        }).expect("format_i32_vec_raw failed to write to string");
    }
    result.push_str(r#"\end{pmatrix}"#);
    result
}

pub fn format_i32_vec(values: &[i32]) -> String {
    math_mode(format_i32_vec_raw(values))
}

pub fn format_i32_vec_labelled_raw(values: &[i32]) -> String {
    let mut result = String::new();
    result.push_str(r#"\begin{pmatrix} "#);
    for (i, &value) in values.iter().enumerate() {
        write!(&mut result, "{}{}{}", value, LABELLED_COMPONENTS[i], {
            if i < values.len() - 1 {
                ",&"
            } else {
                ""
            }
        }).expect("format_i32_vec_labelled_raw failed to write to string");
    }
    result.push_str(r#"\end{pmatrix}"#);
    result
}

pub fn format_i32_vec_labelled(values: &[i32]) -> String {
    math_mode(format_i32_vec_labelled_raw(values))
}

pub fn format_f32_vec_raw_2(values: &[f32]) -> String {
    let mut result = String::new();
    result.push('(');
    for (i, &value) in values.iter().enumerate() {
        if i > 0 {
            result.push_str(", ");
        }
        write!(&mut result, "{}", format_f32_raw(value, None)).expect("format_f32_vec_raw_2 failed to write to string");
    }
    result.push(')');
    result
}

pub fn format_f32_vec_raw(values: &[f32]) -> String {
    let mut result = String::new();
    result.push_str(r#"\begin{pmatrix} "#);
    for (i, &value) in values.iter().enumerate() {
        write!(&mut result, "{}{}", format_f32_raw(value, None), {
            if i < values.len() - 1 {
                ",&"
            } else {
                ""
            }
        }).expect("format_f32_vec_raw failed to write to string");
    }
    result.push_str(r#"\end{pmatrix}"#);
    result
}

pub fn format_f32_vec(values: &[f32]) -> String {
    math_mode(format_f32_vec_raw(values))
}

pub fn format_f32_vec_labelled_raw(values: &[f32]) -> String {
    let mut result = String::new();
    result.push_str(r#"\begin{pmatrix} "#);
    for (i, &value) in values.iter().enumerate() {
        write!(&mut result, "{}{}{}", format_f32_raw(value, None), LABELLED_COMPONENTS[i], {
            if i < values.len() - 1 {
                ",&"
            } else {
                ""
            }
        }).expect("format_f32_vec_labelled_raw failed to write to string");
    }
    result.push_str(r#"\end{pmatrix}"#);
    result
}

pub fn format_f32_vec_labelled(values: &[f32]) -> String {
    math_mode(format_f32_vec_labelled_raw(values))
}

pub fn format_i32_root_raw(power: i32, coeffecient: i32, radicand: i32) -> String {
    match (power, coeffecient, radicand) {
        (1, _, _) => format!("{}", format_i32_raw(coeffecient * radicand)),
        (2, 1, _) => format_i32_raw(radicand),
        (2, _, _) => format!(r#"{coeffecient}\sqrt{{{radicand}}}"#),
        (_, 1, 1) => r#"1"#.to_string(),
        (_, _, 1) => format!(r#"{coeffecient}"#),
        (_, 1, _) => format!(r#"\sqrt[{power}]{{{radicand}}}"#),
        _ => format!(r#"{coeffecient}\sqrt[{power}]{{{radicand}}}"#),
    }
}

pub fn format_i32_root(power: i32, coeffecient: i32, radicand: i32) -> String {
    match (power, coeffecient, radicand) {
        (1, _, _) => format!("{}", format_i32(coeffecient * radicand)),
        (2, 1, _) => format_i32(radicand),
        (2, _, _) => format!(r#"\({coeffecient}\sqrt{{{radicand}}}\)"#),
        (_, 1, 1) => r#"\(1\)"#.to_string(),
        (_, _, 1) => format!(r#"\({coeffecient}\)"#),
        (_, 1, _) => format!(r#"\(\sqrt[{power}]{{{radicand}}}\)"#),
        _ => format!(r#"\({coeffecient}\sqrt[{power}]{{{radicand}}}\)"#),
    }
}

pub fn format_i32_fraction_raw(numerator: i32, denominator: i32) -> String {
    match (numerator, denominator) {
        (0, _) => format_i32_raw(0),
        (_, 1) => format_i32_raw(numerator),
        _ => format!(r#"\frac{{{numerator}}}{{{denominator}}}"#),
    }
}

pub fn format_i32_fraction(numerator: i32, denominator: i32) -> String {
    math_mode(format_i32_fraction_raw(numerator, denominator))
}

pub fn format_random_identifier_raw() -> &'static str {
    LABELLED_IDENTIFIERS.choose(&mut rand::thread_rng()).unwrap()
}

pub fn format_random_identifier() -> String {
    math_mode(format_random_identifier_raw())
}

pub fn format_random_greek_identifier_raw() -> &'static str {
    LABELLED_SYMBOLS.choose(&mut rand::thread_rng()).unwrap()
}

pub fn format_random_greek_identifier() -> String {
    math_mode(format_random_greek_identifier_raw())
}
