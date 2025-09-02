use rand::{
    Rng,
    distributions::{Distribution, Uniform},
};

struct Filter<Dist, Test> {
    dist: Dist,
    test: Test,
}
impl <T, Dist, Test> Distribution<T> for Filter<Dist, Test>
where
    Dist: Distribution<T>,
    Test: Fn(&T) -> bool,
{
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> T {
        loop {
            let x = self.dist.sample(rng);
            if (self.test)(&x) {
                return x;
            }
        }
    }
}

pub fn gen_range_i32(min: i32, max: i32) -> i32 {
    rand::thread_rng().gen_range(min..max)
}

pub fn gen_range_i32_except(min: i32, max: i32, except: i32) -> i32 {
    let mut rng = rand::thread_rng();
    loop {
        let num = rng.gen_range(min..max);
        if num != except {
            return num;
        }
    }
}

pub fn gen_range_i32_except_within_range(min: i32, max: i32, except_min: i32, except_max: i32) -> i32 {
    let dist = Filter {
        dist: Uniform::new(min, max),
        test: |x: &_| x < &except_min || x > &except_max,
    };
    rand::thread_rng().sample(&dist)
}

pub fn gen_range_f32(min: f32, max: f32) -> f32 {
    rand::thread_rng().gen_range(min..max)
}

pub fn gen_range_f32_except(min: f32, max: f32, except: f32) -> f32 {
    let mut rng = rand::thread_rng();
    loop {
        let num = rng.gen_range(min..max);
        if num != except {
            return num;
        }
    }
}

pub fn gen_range_f32_except_within_range(min: f32, max: f32, except_min: f32, except_max: f32) -> f32 {
    let dist = Filter {
        dist: Uniform::new(min, max),
        test: |x: &_| x < &except_min || x > &except_max,
    };
    rand::thread_rng().sample(&dist)
}

pub fn get_particle_object_name() -> (&'static str, &'static str) {
    let mut rng = rand::thread_rng();
    let object_type = [
        ("A", "ball"),
        ("A", "rock"),
        ("A", "car"),
        ("A", "volleyball"),
        ("A", "truck"),
        ("A", "boat"),
        ("A", "plane"),
        ("A", "rocket"),
        ("A", "ship"),
        ("A", "mass"),
    ];
    object_type[rng.gen_range(0..object_type.len())]
}

pub fn get_long_object_name() -> (&'static str, &'static str) {
    let mut rng = rand::thread_rng();
    let object_type = [
        ("A", "beam"),
        ("A", "ladder"),
        ("A", "pole"),
        ("A", "metal pole"),
        ("A", "steel beam"),
        ("A", "stick"),
    ];
    object_type[rng.gen_range(0..object_type.len())]
}

pub fn coin_flip() -> bool {
    rand::thread_rng().gen_bool(0.5)
}
