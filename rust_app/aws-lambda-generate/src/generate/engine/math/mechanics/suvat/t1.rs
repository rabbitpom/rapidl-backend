/*
 * 
 * DAT: 25/05/2024 11:35
 * DES: This question models an object as a particle with constant acceleration and a velocity.
 * ASK: Find velocity of particle at T
 * ASK: Find displacement of particle at T from origin O
 * ASK: Find displacement of particle at T from arbitrary origin
 *
 */

use crate::generate::{
    formatter::{self, LABEL_MS, LABEL_MS_RAW, LABEL_AS, LABEL_AS_RAW, LABEL_M, LABEL_M_RAW},
    helper,
    oncelabel::OnceLabel,
    questionstacker::Stacker,
    question::{Question, QuestionHeader, MarkScheme},
};

pub fn generate() -> Stacker {
    let mut stacker = Stacker::new();
    let mut oncelabel = OnceLabel::new();

    let (p_elision, p_name) = helper::get_particle_object_name();
    let (p_label, p_label_raw) = oncelabel.next_label_raw();
    let t_0 = helper::gen_range_i32(0, 6);
    let t_1 = helper::gen_range_i32_except(0, 6, t_0);

    let (a_i, a_j) = (helper::gen_range_i32_except(-10, 10, 0), helper::gen_range_i32_except(-10, 10, 0));
    let (v_i, v_j) = (helper::gen_range_i32_except(-10, 10, 0), helper::gen_range_i32_except(-10, 10, 0));

    let formatted_a = formatter::format_i32_group_labelled_raw(&[a_i, a_j]);
    let formatted_v = formatter::format_i32_group_labelled_raw(&[v_i, v_j]);

    let formatted_raw_a = formatter::format_i32_group_labelled_raw2(&[a_i, a_j]);
    let formatted_raw_v = formatter::format_i32_group_labelled_raw2(&[v_i, v_j]);

    // (1) Root question body
    let rq_1 = Question::new(
        QuestionHeader::new(
            format!(r#"{p_elision} {p_name}, {p_label_raw}, is modelled as a particle and moves with constant acceleration {formatted_raw_a}{LABEL_AS_RAW}. At time T = {t_0} seconds {p_label_raw} is moving with velocity {formatted_raw_v}{LABEL_MS_RAW}"#),
            format!(r#"{p_elision} {p_name}, \({p_label}\), is modelled as a particle and moves with constant acceleration \({formatted_a}{LABEL_AS}\). At time \(\mathbf{{T}} = {t_0}\) seconds \({p_label}\) is moving with velocity \({formatted_v}{LABEL_MS}\)"#),
        )
    );
    stacker.next_root_question(rq_1);
    
    // (1.a) Sub question 
    let rq_1_a = Question::from_header_and_scheme(
        QuestionHeader::new(
            format!(r#"Find the velocity of {p_label_raw} at T = {t_1} seconds."#), 
            format!(r#"Find the velocity of \({p_label}\) at \(\mathbf{{T}} = {t_1}\) seconds."#)
        ),
        MarkScheme::from(
            format!(r#"Understand the integral of acceleration is velocity, v = I(a) + C. Solving for constant, C, at T = {t_0}, and finding a complete equation for v. Finally, substituting T = {t_1} using the found equation for v to get v = {}{LABEL_MS_RAW}"#, {
                // v = I(a) + C
                let (ic, ij) = (v_i - a_i * t_0, v_j - a_j * t_0);
                let (iv, jv) = (a_i * t_1 + ic, a_j * t_1 + ij);
                formatter::format_i32_group_labelled_raw2(&[iv, jv])
            }),
            format!(r#"Understand the integral of acceleration is velocity, \(\mathbf{{v}}=\int{{\mathbf{{a}}}}\,dt+\mathbf{{c}}\). Solving for constant, \(\mathbf{{c}}\), at \(\mathbf{{T}} = {t_1}\) using the found equation for \(\mathbf{{v}}\) to get \(\mathbf{{v}}={}{LABEL_MS}\)"#, {
                // v = I(a) + C
                let (ic, jc) = (v_i - a_i * t_0, v_j - a_j * t_0);
                let (iv, jv) = (a_i * t_1 + ic, a_j * t_1 + jc);
                formatter::format_i32_group_labelled_raw(&[iv, jv])
            })
        )
    );
    stacker.next_root_sub_question(rq_1_a);

    if helper::coin_flip() {
        if helper::coin_flip() {
            // (1.b) Relative to origin
            let t_2 = helper::gen_range_i32(1, 30);
            let (i_s, j_s) = (helper::gen_range_i32_except(-100, 100, 0), helper::gen_range_i32_except(-100, 100, 0));
            
            let formatted_s = formatter::format_i32_group_labelled_raw(&[i_s, j_s]);

            let formatted_raw_s = formatter::format_i32_group_labelled_raw2(&[i_s, j_s]);

            let rq_1_b = Question::from(
                QuestionHeader::new(
                    format!(r#"The position vector of {p_label_raw} relative to a fixed origin O is {formatted_raw_s}{LABEL_M_RAW} at T = 0."#),
                    format!(r#"The position vector of \({p_label}\) relative to a fixed origin \(\mathbf{{O}}\) is \({formatted_s}{LABEL_M}\) at \(\mathbf{{T}}=0\)."#)
                ),
                format!(r#"Find the position vector of {p_label_raw} relative to O at time T = {t_2} seconds."#),
                format!(r#"Find the position vector of \({p_label}\) relative to \(\mathbf{{O}}\) at time \(\mathbf{{T}}={t_2}\) seconds."#),
                MarkScheme::from(
                    format!(r#"Understand the integral of velocity is displacement, s = I(v) + K. Solving for constant, K, at T = 0, and finding a complete equation for s. Finally, substituting T = {t_2} using the found equation for s to get s = {}{LABEL_M_RAW}"#, {
                        // v = I(a) + c
                        // s = I(v) + k
                        let (ic, jc) = (v_i - a_i * t_0, v_j - a_j * t_0);
                        let (iv0, ic, is, jv0, jc, js, t2) = (a_i as f32, ic as f32, i_s as f32, a_j as f32, jc as f32, j_s as f32, t_2 as f32);
                        let (is, js) = (0.5 * iv0 * t2 * t2 + ic * t2 + is, 0.5 * jv0 * t2 * t2 + jc * t2 + js);
                        formatter::format_f32_group_labelled_raw2(&[is, js])
                    }),
                    format!(r#"Understand the integral of velocity is displacement, \(\mathbf{{s}}=\int{{\mathbf{{v}}}}\,dt+\mathbf{{k}}\). Solving for constant, \(\mathbf{{k}}\), at \(\mathbf{{T}} = {t_2}\) using the found equation for \(\mathbf{{s}}\) to get \(\mathbf{{s}}={}{LABEL_M}\)"#, {
                        // v = I(a) + c
                        // s = I(v) + k
                        let (ic, jc) = (v_i - a_i * t_0, v_j - a_j * t_0);
                        let (iv0, ic, is, jv0, jc, js, t2) = (a_i as f32, ic as f32, i_s as f32, a_j as f32, jc as f32, j_s as f32, t_2 as f32);
                        let (is, js) = (0.5 * iv0 * t2 * t2 + ic * t2 + is, 0.5 * jv0 * t2 * t2 + jc * t2 + js);
                        formatter::format_f32_group_labelled_raw(&[is, js])
                    })
                )
            );
            stacker.next_root_sub_question(rq_1_b);
        } else {
            // (1.b) Relative to a random vector
            let t_2 = helper::gen_range_i32(1, 30);
            let (r_i_s, r_j_s) = (helper::gen_range_i32_except(-100, 100, 0), helper::gen_range_i32_except(-100, 100, 0));
            let (i_s, j_s) = (helper::gen_range_i32_except(-100, 100, 0), helper::gen_range_i32_except(-100, 100, 0));

            let formatted_r_s = formatter::format_i32_group_labelled_raw(&[r_i_s, r_j_s]);
            let formatted_s = formatter::format_i32_group_labelled_raw(&[i_s, j_s]);

            let formatted_raw_r_s = formatter::format_i32_group_labelled_raw2(&[r_i_s, r_j_s]);
            let formatted_raw_s = formatter::format_i32_group_labelled_raw2(&[i_s, j_s]);

            let rq_1_b = Question::from(
                QuestionHeader::new(
                    format!(r#"The position vector of {p_label_raw} relative to {formatted_raw_r_s} is {formatted_raw_s}{LABEL_M_RAW} at T = 0."#),
                    format!(r#"The position vector of \({p_label}\) relative to \({formatted_r_s}\) is \({formatted_s}{LABEL_M}\) at \(\mathbf{{T}}=0\)."#)
                ),
                format!(r#"Find the position vector of {p_label_raw} relative to {formatted_raw_r_s} at time T = {t_2} seconds."#),
                format!(r#"Find the position vector of \({p_label}\) relative to \({formatted_r_s}\) at time \(\mathbf{{T}}={t_2}\) seconds."#),
                MarkScheme::from(
                    format!(r#"Notice how the question is asking for an answer relative to {formatted_raw_r_s}, this means we can ignore it entirely. It would not be the case if it asked for an answer relative to the origin O. Understand the integral of velocity is displacement, s = I(v) + K. Solving for constant, K, at T = 0, and finding a complete equation for s. Finally, substituting T = {t_2} using the found equation for s to get s = {}{LABEL_M_RAW}"#, {
                        // We can safely ignore the random vector, since we're working with
                        // relative vectors here... so this question is actually just the same as
                        // the sub question above LOL!
                        // v = I(a) + c
                        // a = I(v) + k
                        let (ic, jc) = (v_i - a_i * t_0, v_j - a_j * t_0);
                        let (iv0, ic, is, jv0, jc, js, t2) = (a_i as f32, ic as f32, i_s as f32, a_j as f32, jc as f32, j_s as f32, t_2 as f32);
                        let (is, js) = (0.5 * iv0 * t2 * t2 + ic * t2 + is, 0.5 * jv0 * t2 * t2 + jc * t2 + js);
                        formatter::format_f32_group_labelled_raw2(&[is, js])
                    }),
                    format!(r#"Notice how the question is asking for an answer relative to \({formatted_r_s}\), this means we can ignore it entirely. It would not be the case if it asked for an answer relative to the origin \(\mathbf{{O}}\). Understand the integral of velocity is displacement, \(\mathbf{{s}}=\int{{\mathbf{{v}}}}\,dt+\mathbf{{k}}\). Solving for constant, \(\mathbf{{k}}\), at \(\mathbf{{T}} = {t_2}\) using the found equation for \(\mathbf{{s}}\) to get \(\mathbf{{s}}={}{LABEL_M}\)"#, {
                        // We can safely ignore the random vector, since we're working with
                        // relative vectors here... so this question is actually just the same as
                        // the sub question above LOL!
                        // v = I(a) + c
                        // a = I(v) + k
                        let (ic, jc) = (v_i - a_i * t_0, v_j - a_j * t_0);
                        let (iv0, ic, is, jv0, jc, js, t2) = (a_i as f32, ic as f32, i_s as f32, a_j as f32, jc as f32, j_s as f32, t_2 as f32);
                        let (is, js) = (0.5 * iv0 * t2 * t2 + ic * t2 + is, 0.5 * jv0 * t2 * t2 + jc * t2 + js);
                        formatter::format_f32_group_labelled_raw(&[is, js])
                    })
                )
            );
            stacker.next_root_sub_question(rq_1_b);
        }
    }

    stacker
}
