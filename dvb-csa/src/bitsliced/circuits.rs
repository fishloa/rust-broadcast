//! Bitsliced boolean circuits for DVB-CSA2 — **generated code, do not edit**.
//!
//! Regenerate with `python3 tools/gen_circuits.py . && cargo fmt -p dvb-csa`
//! from the `dvb-csa` crate root (the generator emits valid but unformatted
//! Rust; rustfmt makes it match the committed file). It reads [`crate::tables`] — which stays the single source
//! of truth for the cipher — and re-expresses each table as a straight-line,
//! branch-free circuit over [`Word`](super::Word) lanes, so one evaluation
//! covers [`LANES`](super::LANES) independent blocks at once.
//!
//! Two syntheses are tried per table and the cheaper wins: a shared ROBDD (one
//! bitwise multiplexer per node) or an algebraic normal form with shared
//! monomials. The linear tables (`PERM`, the S-box index selection,
//! `csa_stream_b_sel`, `STREAM_OUT`) reduce to rewiring plus a few XORs, and
//! the generator asserts that linearity rather than assuming it.
//!
//! **Nothing here is taken on trust.** `circuit_tests.rs` — deliberately not
//! generated, so regeneration cannot regenerate its own gate — evaluates every
//! circuit over its *entire* input domain and compares against the table it was
//! generated from. A generator bug cannot ship.
#![allow(clippy::identity_op)]

use super::Word;

/// All-zero lane mask — used only where a table column is identically zero.
#[allow(dead_code)]
const ZERO: Word = 0;
/// All-ones lane mask — used only where a table column is identically one.
#[allow(dead_code)]
const ONES: Word = !0;

/// Bitsliced `SBOX[..]` — the block cipher's 8-bit substitution box.
///
/// `x[k]` carries bit `k` of the S-box input for every lane; the
/// result carries bit `k` of `SBOX[input]` for every lane.
///
/// Synthesis: ROBDD, order [0, 1, 5, 7, 4, 2, 3, 6] — 350 gates.
#[inline]
pub(super) fn block_sbox(x: &[Word; 8]) -> [Word; 8] {
    let [x0, x1, x2, x3, x4, x5, x6, x7] = *x;
    let t0 = x3 ^ x2;
    let t1 = !x6 ^ ((!x6 ^ !x3) & x2);
    let t2 = !t0 ^ ((!t0 ^ t1) & x4);
    let t3 = x6 & !x3;
    let t4 = !x6 | !x3;
    let t5 = !t3 ^ ((!t3 ^ t4) & x2);
    let t6 = !x6 & x3;
    let t7 = t6 ^ ((t6 ^ t4) & x2);
    let t8 = !t5 ^ ((!t5 ^ t7) & x4);
    let t9 = t2 ^ ((t2 ^ t8) & x7);
    let t10 = !x6 & !x2;
    let t11 = x2 ^ ((x2 ^ t10) & x4);
    let t12 = x3 ^ ((x3 ^ !x6) & x2);
    let t13 = x6 ^ x3;
    let t14 = t13 & !x2;
    let t15 = !t12 ^ ((!t12 ^ t14) & x4);
    let t16 = t11 ^ ((t11 ^ t15) & x7);
    let t17 = !t9 ^ ((!t9 ^ t16) & x5);
    let t18 = !x3 ^ ((!x3 ^ !x6) & x2);
    let t19 = !t6 ^ ((!t6 ^ !x3) & x2);
    let t20 = !t18 ^ ((!t18 ^ t19) & x4);
    let t21 = !x6 & !x3;
    let t22 = t4 ^ ((t4 ^ t21) & x2);
    let t23 = t22 ^ ((t22 ^ t0) & x4);
    let t24 = t20 ^ ((t20 ^ t23) & x7);
    let t25 = !x6 ^ ((!x6 ^ t21) & x2);
    let t26 = !t21 ^ ((!t21 ^ t4) & x2);
    let t27 = !t25 ^ ((!t25 ^ t26) & x4);
    let t28 = x6 & !x2;
    let t29 = x3 ^ ((x3 ^ t13) & x2);
    let t30 = t28 ^ ((t28 ^ t29) & x4);
    let t31 = t27 ^ ((t27 ^ t30) & x7);
    let t32 = !t24 ^ ((!t24 ^ t31) & x5);
    let t33 = !t17 ^ ((!t17 ^ t32) & x1);
    let t34 = t13 ^ ((t13 ^ t21) & x2);
    let t35 = !t6 ^ x2;
    let t36 = t34 ^ ((t34 ^ t35) & x4);
    let t37 = t3 ^ ((t3 ^ t6) & x2);
    let t38 = !t4 ^ x2;
    let t39 = t37 ^ ((t37 ^ t38) & x4);
    let t40 = !t36 ^ ((!t36 ^ t39) & x7);
    let t41 = !t13 ^ x2;
    let t42 = !t41 ^ x4;
    let t43 = !t21 ^ ((!t21 ^ !x6) & x2);
    let t44 = t3 ^ ((t3 ^ t4) & x2);
    let t45 = !t43 ^ ((!t43 ^ t44) & x4);
    let t46 = !t42 ^ ((!t42 ^ t45) & x7);
    let t47 = t40 ^ ((t40 ^ t46) & x5);
    let t48 = t6 & !x2;
    let t49 = !x3 ^ ((!x3 ^ t6) & x2);
    let t50 = t48 ^ ((t48 ^ t49) & x4);
    let t51 = t3 ^ ((t3 ^ !x3) & x2);
    let t52 = t4 & x2;
    let t53 = !t51 ^ ((!t51 ^ t52) & x4);
    let t54 = !t50 ^ ((!t50 ^ t53) & x7);
    let t55 = t3 & x2;
    let t56 = !t6 & !x2;
    let t57 = !t55 ^ ((!t55 ^ t56) & x4);
    let t58 = t13 ^ ((t13 ^ t3) & x2);
    let t59 = t6 | !x2;
    let t60 = !t58 ^ ((!t58 ^ t59) & x4);
    let t61 = t57 ^ ((t57 ^ t60) & x7);
    let t62 = !t54 ^ ((!t54 ^ t61) & x5);
    let t63 = !t47 ^ ((!t47 ^ t62) & x1);
    let t64 = !t33 ^ ((!t33 ^ t63) & x0);
    let t65 = !x6 | !x2;
    let t66 = t65 ^ ((t65 ^ t29) & x4);
    let t67 = t13 | !x2;
    let t68 = t21 & !x2;
    let t69 = !t67 ^ ((!t67 ^ t68) & x4);
    let t70 = t66 ^ ((t66 ^ t69) & x7);
    let t71 = t13 ^ ((t13 ^ t4) & x2);
    let t72 = !x3 | !x2;
    let t73 = !t71 ^ ((!t71 ^ t72) & x4);
    let t74 = x6 ^ ((x6 ^ t13) & x2);
    let t75 = !t71 ^ ((!t71 ^ t74) & x4);
    let t76 = !t73 ^ ((!t73 ^ t75) & x7);
    let t77 = !t70 ^ ((!t70 ^ t76) & x5);
    let t78 = x3 ^ ((x3 ^ t6) & x2);
    let t79 = !t65 ^ ((!t65 ^ t78) & x4);
    let t80 = !t1 ^ ((!t1 ^ t18) & x4);
    let t81 = !t79 ^ ((!t79 ^ t80) & x7);
    let t82 = t21 ^ ((t21 ^ t6) & x2);
    let t83 = t82 ^ ((t82 ^ t6) & x4);
    let t84 = !t3 & !x2;
    let t85 = t6 ^ ((t6 ^ !x3) & x2);
    let t86 = t84 ^ ((t84 ^ t85) & x4);
    let t87 = t83 ^ ((t83 ^ t86) & x7);
    let t88 = t81 ^ ((t81 ^ t87) & x5);
    let t89 = t77 ^ ((t77 ^ t88) & x1);
    let t90 = t4 ^ ((t4 ^ t13) & x2);
    let t91 = !x6 ^ ((!x6 ^ t4) & x2);
    let t92 = t90 ^ ((t90 ^ t91) & x4);
    let t93 = !x3 ^ ((!x3 ^ t3) & x2);
    let t94 = t21 & x2;
    let t95 = t93 ^ ((t93 ^ t94) & x4);
    let t96 = !t92 ^ ((!t92 ^ t95) & x7);
    let t97 = !x6 ^ ((!x6 ^ t3) & x2);
    let t98 = t97 ^ ((t97 ^ t28) & x4);
    let t99 = t90 ^ ((t90 ^ t58) & x4);
    let t100 = t98 ^ ((t98 ^ t99) & x7);
    let t101 = !t96 ^ ((!t96 ^ t100) & x5);
    let t102 = x6 ^ ((x6 ^ t3) & x2);
    let t103 = !t102 ^ x4;
    let t104 = t21 ^ ((t21 ^ !x6) & x2);
    let t105 = !t4 & !x2;
    let t106 = t104 ^ ((t104 ^ t105) & x4);
    let t107 = t103 ^ ((t103 ^ t106) & x7);
    let t108 = !t21 ^ ((!t21 ^ t6) & x2);
    let t109 = !x3 ^ ((!x3 ^ t13) & x2);
    let t110 = !t108 ^ ((!t108 ^ t109) & x4);
    let t111 = t3 ^ ((t3 ^ t21) & x2);
    let t112 = t111 ^ ((t111 ^ t71) & x4);
    let t113 = t110 ^ ((t110 ^ t112) & x7);
    let t114 = t107 ^ ((t107 ^ t113) & x5);
    let t115 = t101 ^ ((t101 ^ t114) & x1);
    let t116 = !t89 ^ ((!t89 ^ t115) & x0);
    let t117 = t3 & !x2;
    let t118 = !t117 ^ ((!t117 ^ !x2) & x4);
    let t119 = x3 & !x2;
    let t120 = t12 ^ ((t12 ^ t119) & x4);
    let t121 = !t118 ^ ((!t118 ^ t120) & x7);
    let t122 = !t4 ^ ((!t4 ^ t3) & x2);
    let t123 = !t90 ^ ((!t90 ^ t122) & x4);
    let t124 = !t19 ^ ((!t19 ^ !x2) & x4);
    let t125 = !t123 ^ ((!t123 ^ t124) & x7);
    let t126 = t121 ^ ((t121 ^ t125) & x5);
    let t127 = !t102 ^ ((!t102 ^ !x2) & x4);
    let t128 = !x6 ^ ((!x6 ^ t13) & x2);
    let t129 = !t21 ^ ((!t21 ^ t13) & x2);
    let t130 = !t128 ^ ((!t128 ^ t129) & x4);
    let t131 = t127 ^ ((t127 ^ t130) & x7);
    let t132 = !t37 ^ ((!t37 ^ t82) & x4);
    let t133 = !t3 ^ ((!t3 ^ !x3) & x2);
    let t134 = t19 ^ ((t19 ^ t133) & x4);
    let t135 = t132 ^ ((t132 ^ t134) & x7);
    let t136 = !t131 ^ ((!t131 ^ t135) & x5);
    let t137 = t126 ^ ((t126 ^ t136) & x1);
    let t138 = !x3 ^ ((!x3 ^ t4) & x2);
    let t139 = t138 ^ ((t138 ^ t74) & x4);
    let t140 = t21 ^ ((t21 ^ t3) & x2);
    let t141 = !t59 ^ ((!t59 ^ t140) & x4);
    let t142 = t139 ^ ((t139 ^ t141) & x7);
    let t143 = t4 ^ ((t4 ^ !x6) & x2);
    let t144 = !t143 ^ ((!t143 ^ t29) & x4);
    let t145 = t4 & !x2;
    let t146 = !t102 ^ ((!t102 ^ t145) & x4);
    let t147 = t144 ^ ((t144 ^ t146) & x7);
    let t148 = !t142 ^ ((!t142 ^ t147) & x5);
    let t149 = t3 | !x2;
    let t150 = t6 & x2;
    let t151 = t150 ^ ((t150 ^ t22) & x4);
    let t152 = t149 ^ ((t149 ^ t151) & x7);
    let t153 = !t3 ^ x2;
    let t154 = x3 ^ ((x3 ^ t4) & x2);
    let t155 = t153 ^ ((t153 ^ t154) & x4);
    let t156 = t154 ^ ((t154 ^ t43) & x4);
    let t157 = t155 ^ ((t155 ^ t156) & x7);
    let t158 = t152 ^ ((t152 ^ t157) & x5);
    let t159 = t148 ^ ((t148 ^ t158) & x1);
    let t160 = t137 ^ ((t137 ^ t159) & x0);
    let t161 = !t13 ^ ((!t13 ^ t6) & x2);
    let t162 = t21 ^ ((t21 ^ !x3) & x2);
    let t163 = t161 ^ ((t161 ^ t162) & x4);
    let t164 = !t56 ^ ((!t56 ^ t37) & x4);
    let t165 = !t163 ^ ((!t163 ^ t164) & x7);
    let t166 = !t67 & !x4;
    let t167 = t13 & x2;
    let t168 = !t167 ^ ((!t167 ^ t56) & x4);
    let t169 = !t166 ^ ((!t166 ^ t168) & x7);
    let t170 = !t165 ^ ((!t165 ^ t169) & x5);
    let t171 = t138 ^ ((t138 ^ t117) & x4);
    let t172 = t4 ^ ((t4 ^ t3) & x2);
    let t173 = t172 ^ ((t172 ^ t4) & x4);
    let t174 = !t171 ^ ((!t171 ^ t173) & x7);
    let t175 = !t58 ^ ((!t58 ^ t18) & x4);
    let t176 = !t21 ^ ((!t21 ^ t3) & x2);
    let t177 = !t3 ^ ((!t3 ^ !x6) & x2);
    let t178 = t176 ^ ((t176 ^ t177) & x4);
    let t179 = !t175 ^ ((!t175 ^ t178) & x7);
    let t180 = t174 ^ ((t174 ^ t179) & x5);
    let t181 = !t170 ^ ((!t170 ^ t180) & x1);
    let t182 = t4 ^ ((t4 ^ !x3) & x2);
    let t183 = t182 ^ ((t182 ^ t122) & x4);
    let t184 = t6 ^ ((t6 ^ t13) & x2);
    let t185 = t184 ^ ((t184 ^ t72) & x4);
    let t186 = !t183 ^ ((!t183 ^ t185) & x7);
    let t187 = !t67 ^ ((!t67 ^ t6) & x4);
    let t188 = !t104 ^ ((!t104 ^ t13) & x4);
    let t189 = t187 ^ ((t187 ^ t188) & x7);
    let t190 = t186 ^ ((t186 ^ t189) & x5);
    let t191 = !t182 ^ ((!t182 ^ t162) & x4);
    let t192 = t3 ^ ((t3 ^ !x6) & x2);
    let t193 = t192 ^ ((t192 ^ t84) & x4);
    let t194 = !t191 ^ ((!t191 ^ t193) & x7);
    let t195 = !t13 ^ ((!t13 ^ !x3) & x2);
    let t196 = !t195 ^ ((!t195 ^ t55) & x4);
    let t197 = !t29 ^ ((!t29 ^ t176) & x4);
    let t198 = t196 ^ ((t196 ^ t197) & x7);
    let t199 = t194 ^ ((t194 ^ t198) & x5);
    let t200 = !t190 ^ ((!t190 ^ t199) & x1);
    let t201 = !t181 ^ ((!t181 ^ t200) & x0);
    let t202 = !t48 ^ ((!t48 ^ t37) & x4);
    let t203 = t25 ^ ((t25 ^ t68) & x4);
    let t204 = !t202 ^ ((!t202 ^ t203) & x7);
    let t205 = !x3 & x2;
    let t206 = t43 ^ ((t43 ^ t205) & x4);
    let t207 = t21 | !x2;
    let t208 = !t207 ^ ((!t207 ^ t149) & x4);
    let t209 = t206 ^ ((t206 ^ t208) & x7);
    let t210 = t204 ^ ((t204 ^ t209) & x5);
    let t211 = !t4 ^ ((!t4 ^ t6) & x2);
    let t212 = t211 ^ ((t211 ^ t28) & x4);
    let t213 = !t6 ^ ((!t6 ^ t21) & x2);
    let t214 = !t213 ^ ((!t213 ^ t41) & x4);
    let t215 = t212 ^ ((t212 ^ t214) & x7);
    let t216 = t143 ^ ((t143 ^ t162) & x4);
    let t217 = !t28 ^ ((!t28 ^ t129) & x4);
    let t218 = t216 ^ ((t216 ^ t217) & x7);
    let t219 = !t215 ^ ((!t215 ^ t218) & x5);
    let t220 = t210 ^ ((t210 ^ t219) & x1);
    let t221 = !t4 ^ ((!t4 ^ t21) & x2);
    let t222 = t105 ^ ((t105 ^ t221) & x4);
    let t223 = !t22 ^ ((!t22 ^ t5) & x4);
    let t224 = !t222 ^ ((!t222 ^ t223) & x7);
    let t225 = t143 ^ ((t143 ^ t29) & x4);
    let t226 = !t221 ^ ((!t221 ^ t128) & x4);
    let t227 = t225 ^ ((t225 ^ t226) & x7);
    let t228 = !t224 ^ ((!t224 ^ t227) & x5);
    let t229 = x6 ^ ((x6 ^ t6) & x2);
    let t230 = !t5 ^ ((!t5 ^ t229) & x4);
    let t231 = !t140 ^ ((!t140 ^ t91) & x4);
    let t232 = t230 ^ ((t230 ^ t231) & x7);
    let t233 = t161 ^ ((t161 ^ !x6) & x4);
    let t234 = !t37 ^ ((!t37 ^ t5) & x4);
    let t235 = !t233 ^ ((!t233 ^ t234) & x7);
    let t236 = !t232 ^ ((!t232 ^ t235) & x5);
    let t237 = t228 ^ ((t228 ^ t236) & x1);
    let t238 = !t220 ^ ((!t220 ^ t237) & x0);
    let t239 = !t13 ^ ((!t13 ^ t4) & x2);
    let t240 = !x6 & x2;
    let t241 = !t239 ^ ((!t239 ^ t240) & x4);
    let t242 = !t240 ^ ((!t240 ^ t21) & x4);
    let t243 = !t241 ^ ((!t241 ^ t242) & x7);
    let t244 = !x3 & !x2;
    let t245 = t244 ^ ((t244 ^ t172) & x4);
    let t246 = !t14 ^ ((!t14 ^ t195) & x4);
    let t247 = !t245 ^ ((!t245 ^ t246) & x7);
    let t248 = !t243 ^ ((!t243 ^ t247) & x5);
    let t249 = x3 ^ ((x3 ^ t3) & x2);
    let t250 = t221 ^ ((t221 ^ t249) & x4);
    let t251 = !t21 & !x2;
    let t252 = t251 ^ ((t251 ^ t211) & x4);
    let t253 = t250 ^ ((t250 ^ t252) & x7);
    let t254 = !t10 ^ ((!t10 ^ t71) & x4);
    let t255 = t13 ^ ((t13 ^ !x6) & x2);
    let t256 = t255 ^ ((t255 ^ t133) & x4);
    let t257 = !t254 ^ ((!t254 ^ t256) & x7);
    let t258 = !t253 ^ ((!t253 ^ t257) & x5);
    let t259 = !t248 ^ ((!t248 ^ t258) & x1);
    let t260 = !t138 ^ ((!t138 ^ t48) & x4);
    let t261 = t49 ^ ((t49 ^ t74) & x4);
    let t262 = !t260 ^ ((!t260 ^ t261) & x7);
    let t263 = !t13 ^ ((!t13 ^ t3) & x2);
    let t264 = x6 ^ ((x6 ^ !x3) & x2);
    let t265 = !t263 ^ ((!t263 ^ t264) & x4);
    let t266 = !t6 ^ ((!t6 ^ t3) & x2);
    let t267 = !t55 ^ ((!t55 ^ t266) & x4);
    let t268 = t265 ^ ((t265 ^ t267) & x7);
    let t269 = !t262 ^ ((!t262 ^ t268) & x5);
    let t270 = x6 ^ x2;
    let t271 = t6 ^ ((t6 ^ t21) & x2);
    let t272 = !t270 ^ ((!t270 ^ t271) & x4);
    let t273 = t4 ^ ((t4 ^ t150) & x4);
    let t274 = !t272 ^ ((!t272 ^ t273) & x7);
    let t275 = t161 ^ ((t161 ^ t4) & x4);
    let t276 = !t6 ^ ((!t6 ^ !x6) & x2);
    let t277 = !t276 ^ ((!t276 ^ t129) & x4);
    let t278 = !t275 ^ ((!t275 ^ t277) & x7);
    let t279 = !t274 ^ ((!t274 ^ t278) & x5);
    let t280 = !t269 ^ ((!t269 ^ t279) & x1);
    let t281 = t259 ^ ((t259 ^ t280) & x0);
    let t282 = t65 ^ ((t65 ^ t7) & x4);
    let t283 = t91 ^ ((t91 ^ t154) & x4);
    let t284 = t282 ^ ((t282 ^ t283) & x7);
    let t285 = !t13 & !x2;
    let t286 = !t285 ^ ((!t285 ^ t276) & x4);
    let t287 = t264 ^ ((t264 ^ t240) & x4);
    let t288 = t286 ^ ((t286 ^ t287) & x7);
    let t289 = !t284 ^ ((!t284 ^ t288) & x5);
    let t290 = t44 ^ ((t44 ^ t78) & x4);
    let t291 = !t255 ^ ((!t255 ^ t229) & x4);
    let t292 = !t290 ^ ((!t290 ^ t291) & x7);
    let t293 = t38 ^ ((t38 ^ t251) & x4);
    let t294 = t104 ^ ((t104 ^ t221) & x4);
    let t295 = !t293 ^ ((!t293 ^ t294) & x7);
    let t296 = t292 ^ ((t292 ^ t295) & x5);
    let t297 = t289 ^ ((t289 ^ t296) & x1);
    let t298 = t4 | !x2;
    let t299 = t117 ^ ((t117 ^ t298) & x4);
    let t300 = !t195 ^ ((!t195 ^ t143) & x4);
    let t301 = !t299 ^ ((!t299 ^ t300) & x7);
    let t302 = !t37 ^ ((!t37 ^ t104) & x4);
    let t303 = !t21 ^ ((!t21 ^ t172) & x4);
    let t304 = t302 ^ ((t302 ^ t303) & x7);
    let t305 = t301 ^ ((t301 ^ t304) & x5);
    let t306 = t38 ^ ((t38 ^ t68) & x4);
    let t307 = !t6 ^ ((!t6 ^ t13) & x2);
    let t308 = !t307 ^ ((!t307 ^ t119) & x4);
    let t309 = !t306 ^ ((!t306 ^ t308) & x7);
    let t310 = t138 ^ ((t138 ^ t167) & x4);
    let t311 = t4 ^ ((t4 ^ t6) & x2);
    let t312 = !t72 ^ ((!t72 ^ t311) & x4);
    let t313 = !t310 ^ ((!t310 ^ t312) & x7);
    let t314 = !t309 ^ ((!t309 ^ t313) & x5);
    let t315 = !t305 ^ ((!t305 ^ t314) & x1);
    let t316 = !t297 ^ ((!t297 ^ t315) & x0);
    let t317 = !t213 ^ ((!t213 ^ t84) & x4);
    let t318 = !t161 ^ ((!t161 ^ t138) & x4);
    let t319 = t317 ^ ((t317 ^ t318) & x7);
    let t320 = !t21 ^ ((!t21 ^ !x3) & x2);
    let t321 = !t5 ^ ((!t5 ^ t320) & x4);
    let t322 = !t276 & !x4;
    let t323 = t321 ^ ((t321 ^ t322) & x7);
    let t324 = !t319 ^ ((!t319 ^ t323) & x5);
    let t325 = !t21 ^ x2;
    let t326 = !t325 ^ ((!t325 ^ t82) & x4);
    let t327 = !t59 ^ ((!t59 ^ !x6) & x4);
    let t328 = t326 ^ ((t326 ^ t327) & x7);
    let t329 = !t59 ^ ((!t59 ^ t93) & x4);
    let t330 = t5 ^ ((t5 ^ t298) & x4);
    let t331 = !t329 ^ ((!t329 ^ t330) & x7);
    let t332 = t328 ^ ((t328 ^ t331) & x5);
    let t333 = t324 ^ ((t324 ^ t332) & x1);
    let t334 = t3 ^ ((t3 ^ t13) & x2);
    let t335 = !t55 ^ ((!t55 ^ t334) & x4);
    let t336 = t133 ^ ((t133 ^ t52) & x4);
    let t337 = !t335 ^ ((!t335 ^ t336) & x7);
    let t338 = !t122 ^ ((!t122 ^ t213) & x4);
    let t339 = !t143 ^ ((!t143 ^ t182) & x4);
    let t340 = t338 ^ ((t338 ^ t339) & x7);
    let t341 = t337 ^ ((t337 ^ t340) & x5);
    let t342 = t263 ^ ((t263 ^ t34) & x4);
    let t343 = t285 ^ ((t285 ^ t342) & x7);
    let t344 = !t244 ^ ((!t244 ^ t240) & x4);
    let t345 = t12 ^ ((t12 ^ t68) & x4);
    let t346 = !t344 ^ ((!t344 ^ t345) & x7);
    let t347 = !t343 ^ ((!t343 ^ t346) & x5);
    let t348 = t341 ^ ((t341 ^ t347) & x1);
    let t349 = t333 ^ ((t333 ^ t348) & x0);
    [t64, t116, t160, t201, t238, t281, !t316, !t349]
}

/// Bitsliced `STREAM_SBOX[0]` — five A-register index bits in,
/// the `pqzyx` bits at positions 0 and 10 out.
///
/// Synthesis: ROBDD, order [3, 1, 0, 2, 4] — 16 gates.
#[inline]
fn stream_sbox_0(x: &[Word; 5]) -> [Word; 2] {
    let [x0, x1, x2, x3, x4] = *x;
    let t0 = x4 & !x2;
    let t1 = !x4 & !x2;
    let t2 = t0 ^ ((t0 ^ t1) & x0);
    let t3 = x4 ^ x2;
    let t4 = t3 & !x0;
    let t5 = !t2 ^ ((!t2 ^ t4) & x1);
    let t6 = !t3 ^ x0;
    let t7 = !x2 ^ ((!x2 ^ t6) & x1);
    let t8 = t5 ^ ((t5 ^ t7) & x3);
    let t9 = t3 & x0;
    let t10 = !t9 ^ x1;
    let t11 = t0 ^ ((t0 ^ !x2) & x0);
    let t12 = !x4 | !x2;
    let t13 = !t12 ^ ((!t12 ^ t3) & x0);
    let t14 = !t11 ^ ((!t11 ^ t13) & x1);
    let t15 = !t10 ^ ((!t10 ^ t14) & x3);
    [t8, t15]
}

/// Bitsliced `STREAM_SBOX[1]` — five A-register index bits in,
/// the `pqzyx` bits at positions 1 and 11 out.
///
/// Synthesis: ROBDD, order [1, 0, 2, 3, 4] — 16 gates.
#[inline]
fn stream_sbox_1(x: &[Word; 5]) -> [Word; 2] {
    let [x0, x1, x2, x3, x4] = *x;
    let t0 = !x4 & x3;
    let t1 = !t0 ^ x2;
    let t2 = x3 ^ ((x3 ^ t1) & x0);
    let t3 = !x4 & !x3;
    let t4 = x4 ^ x3;
    let t5 = !t3 ^ ((!t3 ^ t4) & x2);
    let t6 = !t5 ^ x0;
    let t7 = !t2 ^ ((!t2 ^ t6) & x1);
    let t8 = !x4 | !x3;
    let t9 = !t8 ^ ((!t8 ^ t4) & x2);
    let t10 = x4 & !x3;
    let t11 = t8 ^ ((t8 ^ t10) & x2);
    let t12 = !t9 ^ ((!t9 ^ t11) & x0);
    let t13 = !t4 ^ ((!t4 ^ t8) & x2);
    let t14 = t10 ^ ((t10 ^ t13) & x0);
    let t15 = t12 ^ ((t12 ^ t14) & x1);
    [t7, t15]
}

/// Bitsliced `STREAM_SBOX[2]` — five A-register index bits in,
/// the `pqzyx` bits at positions 2 and 4 out.
///
/// Synthesis: ROBDD, order [1, 2, 3, 0, 4] — 12 gates.
#[inline]
fn stream_sbox_2(x: &[Word; 5]) -> [Word; 2] {
    let [x0, x1, x2, x3, x4] = *x;
    let t0 = x4 & !x0;
    let t1 = !t0 ^ x3;
    let t2 = !x4 & !x0;
    let t3 = !t2 ^ x3;
    let t4 = !t1 ^ ((!t1 ^ t3) & x2);
    let t5 = !x4 & x0;
    let t6 = !x0 ^ ((!x0 ^ t5) & x3);
    let t7 = !t0 ^ ((!t0 ^ !x0) & x3);
    let t8 = t6 ^ ((t6 ^ t7) & x2);
    let t9 = !t0 ^ ((!t0 ^ !x4) & x3);
    let t10 = !t9 ^ x2;
    let t11 = !t8 ^ ((!t8 ^ t10) & x1);
    [t4, !t11]
}

/// Bitsliced `STREAM_SBOX[3]` — five A-register index bits in,
/// the `pqzyx` bits at positions 3 and 5 out.
///
/// Synthesis: ROBDD, order [4, 1, 2, 0, 3] — 9 gates.
#[inline]
fn stream_sbox_3(x: &[Word; 5]) -> [Word; 2] {
    let [x0, x1, x2, x3, x4] = *x;
    let t0 = !x3 | !x0;
    let t1 = !t0 ^ x2;
    let t2 = x3 ^ x0;
    let t3 = x0 ^ ((x0 ^ t2) & x2);
    let t4 = !t1 ^ ((!t1 ^ t3) & x1);
    let t5 = !x3 ^ ((!x3 ^ !x0) & x2);
    let t6 = !t2 ^ ((!t2 ^ t5) & x1);
    let t7 = t4 ^ ((t4 ^ t6) & x4);
    let t8 = !t6 ^ ((!t6 ^ t4) & x4);
    [t7, !t8]
}

/// Bitsliced `STREAM_SBOX[4]` — five A-register index bits in,
/// the `pqzyx` bits at positions 6 and 8 out.
///
/// Synthesis: ROBDD, order [0, 1, 3, 2, 4] — 16 gates.
#[inline]
fn stream_sbox_4(x: &[Word; 5]) -> [Word; 2] {
    let [x0, x1, x2, x3, x4] = *x;
    let t0 = !x4 & !x2;
    let t1 = !t0 ^ x3;
    let t2 = !t1 ^ ((!t1 ^ !x2) & x1);
    let t3 = !x4 | !x2;
    let t4 = t3 & x3;
    let t5 = x4 ^ x2;
    let t6 = !x4 & x2;
    let t7 = t5 ^ ((t5 ^ t6) & x3);
    let t8 = !t4 ^ ((!t4 ^ t7) & x1);
    let t9 = t2 ^ ((t2 ^ t8) & x0);
    let t10 = t6 & !x3;
    let t11 = t10 | !x1;
    let t12 = t5 ^ ((t5 ^ !x2) & x3);
    let t13 = !t5 ^ ((!t5 ^ t3) & x3);
    let t14 = !t12 ^ ((!t12 ^ t13) & x1);
    let t15 = !t11 ^ ((!t11 ^ t14) & x0);
    [!t9, !t15]
}

/// Bitsliced `STREAM_SBOX[5]` — five A-register index bits in,
/// the `pqzyx` bits at positions 7 and 9 out.
///
/// Synthesis: ROBDD, order [0, 3, 2, 1, 4] — 15 gates.
#[inline]
fn stream_sbox_5(x: &[Word; 5]) -> [Word; 2] {
    let [x0, x1, x2, x3, x4] = *x;
    let t0 = !x4 & x1;
    let t1 = t0 | !x2;
    let t2 = t1 ^ ((t1 ^ !x1) & x3);
    let t3 = !x4 | !x1;
    let t4 = !t3 ^ x2;
    let t5 = !x1 ^ ((!x1 ^ t3) & x2);
    let t6 = !t4 ^ ((!t4 ^ t5) & x3);
    let t7 = !t2 ^ ((!t2 ^ t6) & x0);
    let t8 = x4 ^ x1;
    let t9 = !t8 ^ x2;
    let t10 = !t8 ^ ((!t8 ^ t9) & x3);
    let t11 = !x4 & !x1;
    let t12 = !t11 ^ x2;
    let t13 = t12 ^ ((t12 ^ t4) & x3);
    let t14 = !t10 ^ ((!t10 ^ t13) & x0);
    [t7, t14]
}

/// Bitsliced `STREAM_SBOX[6]` — five A-register index bits in,
/// the `pqzyx` bits at positions 12 and 13 out.
///
/// Synthesis: ROBDD, order [1, 3, 2, 0, 4] — 16 gates.
#[inline]
fn stream_sbox_6(x: &[Word; 5]) -> [Word; 2] {
    let [x0, x1, x2, x3, x4] = *x;
    let t0 = x4 & !x0;
    let t1 = !x4 ^ ((!x4 ^ t0) & x2);
    let t2 = !t0 & !x2;
    let t3 = !t1 ^ ((!t1 ^ t2) & x3);
    let t4 = !x4 & !x0;
    let t5 = !x4 & x0;
    let t6 = t4 ^ ((t4 ^ t5) & x2);
    let t7 = !x4 & !x2;
    let t8 = !t6 ^ ((!t6 ^ t7) & x3);
    let t9 = !t3 ^ ((!t3 ^ t8) & x1);
    let t10 = !x0 ^ ((!x0 ^ t4) & x2);
    let t11 = !t2 ^ ((!t2 ^ t10) & x3);
    let t12 = !t4 & !x2;
    let t13 = t0 ^ ((t0 ^ !x0) & x2);
    let t14 = !t12 ^ ((!t12 ^ t13) & x3);
    let t15 = !t11 ^ ((!t11 ^ t14) & x1);
    [!t9, !t15]
}

/// Bitsliced `STREAM_CDEF[..]` — the C/D/E/F feedback table, ten index
/// bits in, its nine live output bits out (positions
/// [`CDEF_OUT_BITS`]); every other position of the table is zero.
///
/// Synthesis: ROBDD, order [3, 7, 9, 5, 6, 2, 1, 4, 8, 0] — 38 gates.
#[inline]
pub(super) fn stream_cdef(x: &[Word; 10]) -> [Word; 9] {
    let [x0, x1, x2, x3, x4, x5, x6, x7, x8, x9] = *x;
    let t0 = x0 ^ x4;
    let t1 = x1 ^ x5;
    let t2 = x2 ^ x6;
    let t3 = x7 ^ x3;
    let t4 = x0 ^ x8;
    let t5 = !t4 ^ x4;
    let t6 = !x0 ^ ((!x0 ^ t5) & x9);
    let t7 = !x0 | !x8;
    let t8 = !x0 & !x8;
    let t9 = t7 ^ ((t7 ^ t8) & x4);
    let t10 = !t9 ^ x1;
    let t11 = !t10 ^ x5;
    let t12 = !x1 ^ ((!x1 ^ t11) & x9);
    let t13 = t9 | !x1;
    let t14 = !t13 ^ x2;
    let t15 = !t14 ^ x6;
    let t16 = t9 & !x1;
    let t17 = !t16 ^ x2;
    let t18 = !t17 ^ x6;
    let t19 = t15 ^ ((t15 ^ t18) & x5);
    let t20 = !x2 ^ ((!x2 ^ t19) & x9);
    let t21 = t13 | !x2;
    let t22 = t13 & !x2;
    let t23 = t21 ^ ((t21 ^ t22) & x6);
    let t24 = t16 | !x2;
    let t25 = t16 & !x2;
    let t26 = t24 ^ ((t24 ^ t25) & x6);
    let t27 = t23 ^ ((t23 ^ t26) & x5);
    let t28 = t27 | !x9;
    let t29 = t27 & x9;
    let t30 = !t28 ^ ((!t28 ^ t29) & x7);
    let t31 = !t30 ^ x3;
    let t32 = x8 & !x9;
    let t33 = !x8 ^ ((!x8 ^ t27) & x9);
    let t34 = !t32 ^ ((!t32 ^ t33) & x7);
    let t35 = !x8 & !x9;
    let t36 = t33 ^ ((t33 ^ t35) & x7);
    let t37 = t34 ^ ((t34 ^ t36) & x3);
    [t0, t1, t2, t3, !t6, !t12, !t20, !t31, !t37]
}

/// Bit destinations of the block cipher's `PERM` table.
///
/// `PERM` is GF(2)-linear and maps each input bit to exactly one output
/// bit, so bitsliced it is free: `PERM[s]` bit `PERM_BIT[k]` is `s` bit
/// `k`.
pub(super) const PERM_BIT: [usize; 8] = [1, 7, 5, 4, 2, 6, 0, 3];

/// `pqzyx` bit positions written by [`stream_sboxes`], in the order it
/// returns them.
pub(super) const STREAM_SBOX_OUT_BITS: [usize; 14] = [0, 10, 1, 11, 2, 4, 3, 5, 6, 8, 7, 9, 12, 13];

/// `cfed` bit positions written by [`stream_cdef`], in the order it
/// returns them.
pub(super) const CDEF_OUT_BITS: [usize; 9] = [0, 1, 2, 3, 8, 9, 10, 11, 12];

/// Bitsliced `csa_stream_sboxes` — the A register in, the fourteen
/// `pqzyx` bits out, ordered as [`STREAM_SBOX_OUT_BITS`].
///
/// The index of each S-box is a GF(2)-linear function of the A bits
/// (`STREAM_SBOX_SEL`'s mask-and-shift chain), so selecting it costs
/// only XORs.
#[inline]
pub(super) fn stream_sboxes(a: &[Word; 40]) -> [Word; 14] {
    let s0_0 = a[36];
    let s0_1 = a[31];
    let s0_2 = a[25];
    let s0_3 = a[6];
    let s0_4 = a[16];
    let s1_0 = a[27] ^ a[37];
    let s1_1 = a[28];
    let s1_2 = a[27];
    let s1_3 = a[14] ^ a[28];
    let s1_4 = a[9];
    let s2_0 = a[26];
    let s2_1 = a[23];
    let s2_2 = a[7] ^ a[21];
    let s2_3 = a[8];
    let s2_4 = a[7] ^ a[23] ^ a[26];
    let s3_0 = a[11] ^ a[32];
    let s3_1 = a[18];
    let s3_2 = a[11];
    let s3_3 = a[5];
    let s3_4 = a[15];
    let s4_0 = a[22] ^ a[38];
    let s4_1 = a[19] ^ a[33];
    let s4_2 = a[24];
    let s4_3 = a[19];
    let s4_4 = a[22];
    let s5_0 = a[39];
    let s5_1 = a[30];
    let s5_2 = a[20];
    let s5_3 = a[17];
    let s5_4 = a[13];
    let s6_0 = a[35];
    let s6_1 = a[10] ^ a[34];
    let s6_2 = a[29] ^ a[35];
    let s6_3 = a[12];
    let s6_4 = a[10];
    let o0 = stream_sbox_0(&[s0_0, s0_1, s0_2, s0_3, s0_4]);
    let o1 = stream_sbox_1(&[s1_0, s1_1, s1_2, s1_3, s1_4]);
    let o2 = stream_sbox_2(&[s2_0, s2_1, s2_2, s2_3, s2_4]);
    let o3 = stream_sbox_3(&[s3_0, s3_1, s3_2, s3_3, s3_4]);
    let o4 = stream_sbox_4(&[s4_0, s4_1, s4_2, s4_3, s4_4]);
    let o5 = stream_sbox_5(&[s5_0, s5_1, s5_2, s5_3, s5_4]);
    let o6 = stream_sbox_6(&[s6_0, s6_1, s6_2, s6_3, s6_4]);
    [
        o0[0], o0[1], o1[0], o1[1], o2[0], o2[1], o3[0], o3[1], o4[0], o4[1], o5[0], o5[1], o6[0],
        o6[1],
    ]
}

/// Bitsliced `csa_stream_b_sel` — four bits XOR-folded out of the B
/// register. Linear, so it is pure XOR.
#[inline]
pub(super) fn stream_b_sel(b: &[Word; 40]) -> [Word; 4] {
    let o0 = b[13] ^ b[27] ^ b[32] ^ b[38];
    let o1 = b[16] ^ b[21] ^ b[23] ^ b[34];
    let o2 = b[15] ^ b[18] ^ b[24] ^ b[33];
    let o3 = b[12] ^ b[25] ^ b[30] ^ b[39];
    [o0, o1, o2, o3]
}

/// Bitsliced `STREAM_OUT[d]` — the two keystream bits a round yields.
///
/// Every entry of `STREAM_OUT` is a 2-bit value replicated across the
/// byte, and each of those bits is linear in the D nibble, so a round's
/// keystream contribution is two XORs. Returns `[high, low]`.
#[inline]
pub(super) fn stream_out(d: &[Word; 4]) -> [Word; 2] {
    let hi = d[2] ^ d[3];
    let lo = d[0] ^ d[1];
    [hi, lo]
}
