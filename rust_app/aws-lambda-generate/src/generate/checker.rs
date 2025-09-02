pub fn is_i32_zero(v: i32) -> bool {
    v == 0
}

pub fn is_f32_zero(v: f32) -> bool {
    v.abs() <= f32::EPSILON
}

pub fn is_valid_frac_i32(_numerator: i32, denominator: i32) -> bool {
    !is_i32_zero(denominator)
}

pub fn is_valid_root_i32(radicand: i32) -> bool {
    radicand >= 0
}
