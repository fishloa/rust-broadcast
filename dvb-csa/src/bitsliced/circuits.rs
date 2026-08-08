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
/// Synthesis: ROBDD, order [0, 1, 2, 5, 3, 7, 4, 6] — 402 gates.
#[inline]
pub(super) fn block_sbox(x: &[Word; 8]) -> [Word; 8] {
    let [x0, x1, x2, x3, x4, x5, x6, x7] = *x;
    let t0 = ONES & x6;
    let t1 = t0 & x4;
    let t2 = ONES & !x6;
    let t3 = t2 | x4;
    let t4 = t1 ^ ((t1 ^ t3) & x7);
    let t5 = t0 | !x4;
    let t6 = t4 ^ ((t4 ^ t5) & x3);
    let t7 = t2 & x4;
    let t8 = t7 ^ ((t7 ^ t5) & x7);
    let t9 = t8 ^ ((t8 ^ t7) & x3);
    let t10 = t6 ^ ((t6 ^ t9) & x5);
    let t11 = ONES & !x4;
    let t12 = ONES & x4;
    let t13 = t2 ^ ((t2 ^ t0) & x4);
    let t14 = t12 ^ ((t12 ^ t13) & x7);
    let t15 = t11 ^ ((t11 ^ t14) & x3);
    let t16 = t0 & !x4;
    let t17 = t11 ^ ((t11 ^ t16) & x7);
    let t18 = t15 ^ ((t15 ^ t17) & x5);
    let t19 = t10 ^ ((t10 ^ t18) & x2);
    let t20 = t12 ^ ((t12 ^ t11) & x7);
    let t21 = t5 ^ ((t5 ^ t3) & x7);
    let t22 = t20 ^ ((t20 ^ t21) & x3);
    let t23 = t2 ^ ((t2 ^ t3) & x7);
    let t24 = t2 & !x4;
    let t25 = t23 ^ ((t23 ^ t24) & x3);
    let t26 = t22 ^ ((t22 ^ t25) & x5);
    let t27 = t0 | x4;
    let t28 = t27 ^ ((t27 ^ t3) & x7);
    let t29 = t16 & !x7;
    let t30 = t28 ^ ((t28 ^ t29) & x3);
    let t31 = t2 | !x4;
    let t32 = t24 ^ ((t24 ^ t31) & x7);
    let t33 = t1 ^ ((t1 ^ t5) & x7);
    let t34 = t32 ^ ((t32 ^ t33) & x3);
    let t35 = t30 ^ ((t30 ^ t34) & x5);
    let t36 = t26 ^ ((t26 ^ t35) & x2);
    let t37 = t19 ^ ((t19 ^ t36) & x1);
    let t38 = t13 ^ ((t13 ^ t31) & x7);
    let t39 = t28 ^ ((t28 ^ t38) & x3);
    let t40 = t0 ^ ((t0 ^ t2) & x4);
    let t41 = t13 | x7;
    let t42 = t40 ^ ((t40 ^ t41) & x3);
    let t43 = t39 ^ ((t39 ^ t42) & x5);
    let t44 = t24 ^ ((t24 ^ t11) & x7);
    let t45 = t7 ^ ((t7 ^ t0) & x7);
    let t46 = t44 ^ ((t44 ^ t45) & x3);
    let t47 = t13 ^ ((t13 ^ t24) & x7);
    let t48 = t40 ^ ((t40 ^ t13) & x7);
    let t49 = t47 ^ ((t47 ^ t48) & x3);
    let t50 = t46 ^ ((t46 ^ t49) & x5);
    let t51 = t43 ^ ((t43 ^ t50) & x2);
    let t52 = t12 ^ ((t12 ^ t27) & x7);
    let t53 = t24 ^ ((t24 ^ t12) & x7);
    let t54 = t52 ^ ((t52 ^ t53) & x3);
    let t55 = t3 | !x7;
    let t56 = t5 ^ ((t5 ^ t27) & x7);
    let t57 = t55 ^ ((t55 ^ t56) & x3);
    let t58 = t54 ^ ((t54 ^ t57) & x5);
    let t59 = t11 & x7;
    let t60 = t7 ^ ((t7 ^ t1) & x7);
    let t61 = t59 ^ ((t59 ^ t60) & x3);
    let t62 = t11 ^ ((t11 ^ t31) & x7);
    let t63 = t24 ^ ((t24 ^ t62) & x3);
    let t64 = t61 ^ ((t61 ^ t63) & x5);
    let t65 = t58 ^ ((t58 ^ t64) & x2);
    let t66 = t51 ^ ((t51 ^ t65) & x1);
    let t67 = t37 ^ ((t37 ^ t66) & x0);
    let t68 = t11 ^ ((t11 ^ t7) & x7);
    let t69 = ONES & !x7;
    let t70 = t68 ^ ((t68 ^ t69) & x3);
    let t71 = t3 ^ ((t3 ^ t40) & x7);
    let t72 = t27 ^ ((t27 ^ t2) & x7);
    let t73 = t71 ^ ((t71 ^ t72) & x3);
    let t74 = t70 ^ ((t70 ^ t73) & x5);
    let t75 = t2 ^ ((t2 ^ t16) & x7);
    let t76 = t47 ^ ((t47 ^ t75) & x3);
    let t77 = t12 ^ ((t12 ^ t31) & x7);
    let t78 = t16 ^ ((t16 ^ t13) & x7);
    let t79 = t77 ^ ((t77 ^ t78) & x3);
    let t80 = t76 ^ ((t76 ^ t79) & x5);
    let t81 = t74 ^ ((t74 ^ t80) & x2);
    let t82 = t24 & x7;
    let t83 = t12 ^ ((t12 ^ t3) & x7);
    let t84 = t82 ^ ((t82 ^ t83) & x3);
    let t85 = t5 ^ ((t5 ^ t1) & x7);
    let t86 = t27 ^ ((t27 ^ t85) & x3);
    let t87 = t84 ^ ((t84 ^ t86) & x5);
    let t88 = t16 ^ ((t16 ^ t5) & x7);
    let t89 = t40 ^ ((t40 ^ t1) & x7);
    let t90 = t88 ^ ((t88 ^ t89) & x3);
    let t91 = t11 | !x7;
    let t92 = t0 | x7;
    let t93 = t91 ^ ((t91 ^ t92) & x3);
    let t94 = t90 ^ ((t90 ^ t93) & x5);
    let t95 = t87 ^ ((t87 ^ t94) & x2);
    let t96 = t81 ^ ((t81 ^ t95) & x1);
    let t97 = t31 ^ ((t31 ^ t12) & x7);
    let t98 = t2 | x7;
    let t99 = t97 ^ ((t97 ^ t98) & x3);
    let t100 = t13 ^ ((t13 ^ t5) & x7);
    let t101 = t13 ^ ((t13 ^ t2) & x7);
    let t102 = t100 ^ ((t100 ^ t101) & x3);
    let t103 = t99 ^ ((t99 ^ t102) & x5);
    let t104 = t27 ^ ((t27 ^ t13) & x7);
    let t105 = t104 ^ ((t104 ^ t98) & x3);
    let t106 = t16 ^ ((t16 ^ t0) & x7);
    let t107 = t106 ^ ((t106 ^ t82) & x3);
    let t108 = t105 ^ ((t105 ^ t107) & x5);
    let t109 = t103 ^ ((t103 ^ t108) & x2);
    let t110 = t13 ^ ((t13 ^ t1) & x7);
    let t111 = t47 ^ ((t47 ^ t110) & x3);
    let t112 = t3 ^ ((t3 ^ t0) & x7);
    let t113 = t7 & x7;
    let t114 = t112 ^ ((t112 ^ t113) & x3);
    let t115 = t111 ^ ((t111 ^ t114) & x5);
    let t116 = t11 ^ ((t11 ^ t24) & x7);
    let t117 = t47 ^ ((t47 ^ t116) & x3);
    let t118 = t40 ^ ((t40 ^ t7) & x7);
    let t119 = t21 ^ ((t21 ^ t118) & x3);
    let t120 = t117 ^ ((t117 ^ t119) & x5);
    let t121 = t115 ^ ((t115 ^ t120) & x2);
    let t122 = t109 ^ ((t109 ^ t121) & x1);
    let t123 = t96 ^ ((t96 ^ t122) & x0);
    let t124 = ONES & x7;
    let t125 = t29 ^ ((t29 ^ t124) & x3);
    let t126 = t12 | !x7;
    let t127 = t126 ^ ((t126 ^ t23) & x3);
    let t128 = t125 ^ ((t125 ^ t127) & x5);
    let t129 = t12 ^ ((t12 ^ t24) & x7);
    let t130 = t40 & !x7;
    let t131 = t3 ^ ((t3 ^ t11) & x7);
    let t132 = t130 ^ ((t130 ^ t131) & x3);
    let t133 = t129 ^ ((t129 ^ t132) & x5);
    let t134 = t128 ^ ((t128 ^ t133) & x2);
    let t135 = t16 ^ ((t16 ^ t2) & x7);
    let t136 = t16 ^ ((t16 ^ t24) & x7);
    let t137 = t135 ^ ((t135 ^ t136) & x3);
    let t138 = t2 ^ ((t2 ^ t31) & x7);
    let t139 = t11 ^ ((t11 ^ t27) & x7);
    let t140 = t138 ^ ((t138 ^ t139) & x3);
    let t141 = t137 ^ ((t137 ^ t140) & x5);
    let t142 = t27 ^ ((t27 ^ t40) & x7);
    let t143 = t142 ^ ((t142 ^ t14) & x3);
    let t144 = t11 | x7;
    let t145 = t144 ^ ((t144 ^ t130) & x3);
    let t146 = t143 ^ ((t143 ^ t145) & x5);
    let t147 = t141 ^ ((t141 ^ t146) & x2);
    let t148 = t134 ^ ((t134 ^ t147) & x1);
    let t149 = t31 | x7;
    let t150 = t8 ^ ((t8 ^ t149) & x3);
    let t151 = t3 & x7;
    let t152 = t151 ^ ((t151 ^ t72) & x3);
    let t153 = t150 ^ ((t150 ^ t152) & x5);
    let t154 = t0 ^ ((t0 ^ t3) & x7);
    let t155 = t7 ^ ((t7 ^ t154) & x3);
    let t156 = t0 ^ ((t0 ^ t24) & x7);
    let t157 = t40 ^ ((t40 ^ t11) & x7);
    let t158 = t156 ^ ((t156 ^ t157) & x3);
    let t159 = t155 ^ ((t155 ^ t158) & x5);
    let t160 = t153 ^ ((t153 ^ t159) & x2);
    let t161 = t7 | !x7;
    let t162 = t126 ^ ((t126 ^ t161) & x3);
    let t163 = t24 ^ ((t24 ^ t1) & x7);
    let t164 = t163 | x3;
    let t165 = t162 ^ ((t162 ^ t164) & x5);
    let t166 = t0 ^ ((t0 ^ t7) & x7);
    let t167 = t166 ^ ((t166 ^ t82) & x3);
    let t168 = t27 ^ ((t27 ^ t31) & x7);
    let t169 = t7 ^ ((t7 ^ t2) & x7);
    let t170 = t168 ^ ((t168 ^ t169) & x3);
    let t171 = t167 ^ ((t167 ^ t170) & x5);
    let t172 = t165 ^ ((t165 ^ t171) & x2);
    let t173 = t160 ^ ((t160 ^ t172) & x1);
    let t174 = t148 ^ ((t148 ^ t173) & x0);
    let t175 = t16 ^ ((t16 ^ t27) & x7);
    let t176 = t138 ^ ((t138 ^ t175) & x3);
    let t177 = t5 | !x7;
    let t178 = t177 | !x3;
    let t179 = t176 ^ ((t176 ^ t178) & x5);
    let t180 = t12 ^ ((t12 ^ t163) & x3);
    let t181 = t27 ^ ((t27 ^ t24) & x7);
    let t182 = t3 ^ ((t3 ^ t16) & x7);
    let t183 = t181 ^ ((t181 ^ t182) & x3);
    let t184 = t180 ^ ((t180 ^ t183) & x5);
    let t185 = t179 ^ ((t179 ^ t184) & x2);
    let t186 = t5 & !x7;
    let t187 = t0 & x7;
    let t188 = t186 ^ ((t186 ^ t187) & x3);
    let t189 = t3 ^ ((t3 ^ t13) & x7);
    let t190 = t189 ^ ((t189 ^ t29) & x3);
    let t191 = t188 ^ ((t188 ^ t190) & x5);
    let t192 = t24 ^ ((t24 ^ t5) & x7);
    let t193 = t116 ^ ((t116 ^ t192) & x3);
    let t194 = t2 ^ ((t2 ^ t13) & x7);
    let t195 = t31 ^ ((t31 ^ t5) & x7);
    let t196 = t194 ^ ((t194 ^ t195) & x3);
    let t197 = t193 ^ ((t193 ^ t196) & x5);
    let t198 = t191 ^ ((t191 ^ t197) & x2);
    let t199 = t185 ^ ((t185 ^ t198) & x1);
    let t200 = t13 ^ ((t13 ^ t16) & x7);
    let t201 = t11 ^ ((t11 ^ t200) & x3);
    let t202 = t2 | !x7;
    let t203 = t202 ^ ((t202 ^ t85) & x3);
    let t204 = t201 ^ ((t201 ^ t203) & x5);
    let t205 = t5 ^ ((t5 ^ t24) & x7);
    let t206 = t27 & x7;
    let t207 = t205 ^ ((t205 ^ t206) & x3);
    let t208 = t72 ^ ((t72 ^ t13) & x3);
    let t209 = t207 ^ ((t207 ^ t208) & x5);
    let t210 = t204 ^ ((t204 ^ t209) & x2);
    let t211 = t5 ^ ((t5 ^ t40) & x7);
    let t212 = t3 ^ ((t3 ^ t12) & x7);
    let t213 = t211 ^ ((t211 ^ t212) & x3);
    let t214 = t88 ^ ((t88 ^ t53) & x3);
    let t215 = t213 ^ ((t213 ^ t214) & x5);
    let t216 = t116 ^ ((t116 ^ t129) & x3);
    let t217 = t1 ^ ((t1 ^ t13) & x7);
    let t218 = t217 ^ ((t217 ^ t17) & x3);
    let t219 = t216 ^ ((t216 ^ t218) & x5);
    let t220 = t215 ^ ((t215 ^ t219) & x2);
    let t221 = t210 ^ ((t210 ^ t220) & x1);
    let t222 = t199 ^ ((t199 ^ t221) & x0);
    let t223 = t5 ^ ((t5 ^ t0) & x7);
    let t224 = t223 ^ ((t223 ^ t175) & x3);
    let t225 = t131 ^ ((t131 ^ t20) & x3);
    let t226 = t224 ^ ((t224 ^ t225) & x5);
    let t227 = t139 ^ ((t139 ^ t149) & x3);
    let t228 = t27 ^ ((t27 ^ t12) & x7);
    let t229 = t135 ^ ((t135 ^ t228) & x3);
    let t230 = t227 ^ ((t227 ^ t229) & x5);
    let t231 = t226 ^ ((t226 ^ t230) & x2);
    let t232 = t1 ^ ((t1 ^ t7) & x7);
    let t233 = t0 ^ ((t0 ^ t13) & x7);
    let t234 = t232 ^ ((t232 ^ t233) & x3);
    let t235 = t1 ^ ((t1 ^ t40) & x7);
    let t236 = t27 ^ ((t27 ^ t16) & x7);
    let t237 = t235 ^ ((t235 ^ t236) & x3);
    let t238 = t234 ^ ((t234 ^ t237) & x5);
    let t239 = t187 ^ ((t187 ^ t32) & x3);
    let t240 = t16 ^ ((t16 ^ t7) & x7);
    let t241 = t27 ^ ((t27 ^ t1) & x7);
    let t242 = t240 ^ ((t240 ^ t241) & x3);
    let t243 = t239 ^ ((t239 ^ t242) & x5);
    let t244 = t238 ^ ((t238 ^ t243) & x2);
    let t245 = t231 ^ ((t231 ^ t244) & x1);
    let t246 = t5 & x7;
    let t247 = t246 ^ ((t246 ^ t156) & x3);
    let t248 = t3 ^ ((t3 ^ t2) & x7);
    let t249 = t62 ^ ((t62 ^ t248) & x3);
    let t250 = t247 ^ ((t247 ^ t249) & x5);
    let t251 = t7 ^ ((t7 ^ t24) & x7);
    let t252 = t1 & x7;
    let t253 = t251 ^ ((t251 ^ t252) & x3);
    let t254 = t13 ^ ((t13 ^ t0) & x7);
    let t255 = t254 ^ ((t254 ^ t138) & x3);
    let t256 = t253 ^ ((t253 ^ t255) & x5);
    let t257 = t250 ^ ((t250 ^ t256) & x2);
    let t258 = t31 ^ ((t31 ^ t1) & x7);
    let t259 = t194 ^ ((t194 ^ t258) & x3);
    let t260 = t0 ^ ((t0 ^ t2) & x7);
    let t261 = t260 ^ ((t260 ^ t41) & x3);
    let t262 = t259 ^ ((t259 ^ t261) & x5);
    let t263 = t16 | !x7;
    let t264 = t263 ^ ((t263 ^ t110) & x3);
    let t265 = t5 | x7;
    let t266 = t0 ^ ((t0 ^ t40) & x7);
    let t267 = t265 ^ ((t265 ^ t266) & x3);
    let t268 = t264 ^ ((t264 ^ t267) & x5);
    let t269 = t262 ^ ((t262 ^ t268) & x2);
    let t270 = t257 ^ ((t257 ^ t269) & x1);
    let t271 = t245 ^ ((t245 ^ t270) & x0);
    let t272 = t3 ^ ((t3 ^ t31) & x7);
    let t273 = t27 ^ ((t27 ^ t11) & x7);
    let t274 = t272 ^ ((t272 ^ t273) & x3);
    let t275 = t0 | !x7;
    let t276 = t275 ^ ((t275 ^ t169) & x3);
    let t277 = t274 ^ ((t274 ^ t276) & x5);
    let t278 = t211 ^ ((t211 ^ t200) & x3);
    let t279 = t1 & !x7;
    let t280 = t12 & x7;
    let t281 = t279 ^ ((t279 ^ t280) & x3);
    let t282 = t278 ^ ((t278 ^ t281) & x5);
    let t283 = t277 ^ ((t277 ^ t282) & x2);
    let t284 = t24 ^ ((t24 ^ t7) & x7);
    let t285 = t55 ^ ((t55 ^ t284) & x3);
    let t286 = t2 ^ ((t2 ^ t40) & x7);
    let t287 = t13 ^ ((t13 ^ t3) & x7);
    let t288 = t286 ^ ((t286 ^ t287) & x3);
    let t289 = t285 ^ ((t285 ^ t288) & x5);
    let t290 = t40 | x7;
    let t291 = t290 ^ ((t290 ^ t177) & x3);
    let t292 = t1 ^ ((t1 ^ t24) & x7);
    let t293 = t151 ^ ((t151 ^ t292) & x3);
    let t294 = t291 ^ ((t291 ^ t293) & x5);
    let t295 = t289 ^ ((t289 ^ t294) & x2);
    let t296 = t283 ^ ((t283 ^ t295) & x1);
    let t297 = t177 ^ ((t177 ^ t1) & x3);
    let t298 = t2 & !x7;
    let t299 = t298 ^ ((t298 ^ t118) & x3);
    let t300 = t297 ^ ((t297 ^ t299) & x5);
    let t301 = t1 | !x7;
    let t302 = t301 ^ ((t301 ^ t248) & x3);
    let t303 = t16 ^ ((t16 ^ t40) & x7);
    let t304 = t303 ^ ((t303 ^ t12) & x3);
    let t305 = t302 ^ ((t302 ^ t304) & x5);
    let t306 = t300 ^ ((t300 ^ t305) & x2);
    let t307 = t2 ^ ((t2 ^ t27) & x7);
    let t308 = t53 ^ ((t53 ^ t307) & x3);
    let t309 = t16 ^ ((t16 ^ t1) & x7);
    let t310 = t309 ^ ((t309 ^ t287) & x3);
    let t311 = t308 ^ ((t308 ^ t310) & x5);
    let t312 = t40 ^ ((t40 ^ t12) & x7);
    let t313 = t312 ^ ((t312 ^ t106) & x3);
    let t314 = t11 ^ ((t11 ^ t0) & x7);
    let t315 = t314 ^ ((t314 ^ t266) & x3);
    let t316 = t313 ^ ((t313 ^ t315) & x5);
    let t317 = t311 ^ ((t311 ^ t316) & x2);
    let t318 = t306 ^ ((t306 ^ t317) & x1);
    let t319 = t296 ^ ((t296 ^ t318) & x0);
    let t320 = t1 ^ ((t1 ^ t16) & x7);
    let t321 = t52 ^ ((t52 ^ t320) & x3);
    let t322 = t236 ^ ((t236 ^ t200) & x3);
    let t323 = t321 ^ ((t321 ^ t322) & x5);
    let t324 = t29 ^ ((t29 ^ t0) & x3);
    let t325 = t31 ^ ((t31 ^ t7) & x7);
    let t326 = t31 ^ ((t31 ^ t325) & x3);
    let t327 = t324 ^ ((t324 ^ t326) & x5);
    let t328 = t323 ^ ((t323 ^ t327) & x2);
    let t329 = t189 ^ ((t189 ^ t314) & x3);
    let t330 = t31 ^ ((t31 ^ t24) & x7);
    let t331 = t330 ^ ((t330 ^ t163) & x3);
    let t332 = t329 ^ ((t329 ^ t331) & x5);
    let t333 = t12 ^ ((t12 ^ t16) & x7);
    let t334 = t333 ^ ((t333 ^ t266) & x3);
    let t335 = t12 ^ ((t12 ^ t2) & x7);
    let t336 = t335 ^ ((t335 ^ t181) & x3);
    let t337 = t334 ^ ((t334 ^ t336) & x5);
    let t338 = t332 ^ ((t332 ^ t337) & x2);
    let t339 = t328 ^ ((t328 ^ t338) & x1);
    let t340 = t24 ^ ((t24 ^ t27) & x7);
    let t341 = t11 ^ ((t11 ^ t2) & x7);
    let t342 = t340 ^ ((t340 ^ t341) & x3);
    let t343 = t307 ^ ((t307 ^ t62) & x3);
    let t344 = t342 ^ ((t342 ^ t343) & x5);
    let t345 = t5 ^ ((t5 ^ t31) & x7);
    let t346 = t68 ^ ((t68 ^ t345) & x3);
    let t347 = t31 ^ ((t31 ^ t0) & x7);
    let t348 = t347 ^ ((t347 ^ t157) & x3);
    let t349 = t346 ^ ((t346 ^ t348) & x5);
    let t350 = t344 ^ ((t344 ^ t349) & x2);
    let t351 = t186 ^ ((t186 ^ t3) & x3);
    let t352 = t11 ^ ((t11 ^ t246) & x3);
    let t353 = t351 ^ ((t351 ^ t352) & x5);
    let t354 = t129 ^ ((t129 ^ t236) & x3);
    let t355 = t2 ^ ((t2 ^ t1) & x7);
    let t356 = t265 ^ ((t265 ^ t355) & x3);
    let t357 = t354 ^ ((t354 ^ t356) & x5);
    let t358 = t353 ^ ((t353 ^ t357) & x2);
    let t359 = t350 ^ ((t350 ^ t358) & x1);
    let t360 = t339 ^ ((t339 ^ t359) & x0);
    let t361 = t7 ^ ((t7 ^ t27) & x7);
    let t362 = t3 ^ ((t3 ^ t24) & x7);
    let t363 = t361 ^ ((t361 ^ t362) & x3);
    let t364 = t98 ^ ((t98 ^ t139) & x3);
    let t365 = t363 ^ ((t363 ^ t364) & x5);
    let t366 = t16 | x7;
    let t367 = t11 ^ ((t11 ^ t40) & x7);
    let t368 = t366 ^ ((t366 ^ t367) & x3);
    let t369 = t11 ^ ((t11 ^ t3) & x7);
    let t370 = t369 ^ ((t369 ^ t3) & x3);
    let t371 = t368 ^ ((t368 ^ t370) & x5);
    let t372 = t365 ^ ((t365 ^ t371) & x2);
    let t373 = t0 ^ ((t0 ^ t5) & x7);
    let t374 = t373 ^ ((t373 ^ t177) & x3);
    let t375 = t333 & !x3;
    let t376 = t374 ^ ((t374 ^ t375) & x5);
    let t377 = t3 ^ ((t3 ^ t1) & x7);
    let t378 = t377 ^ ((t377 ^ t217) & x3);
    let t379 = t186 ^ ((t186 ^ t106) & x3);
    let t380 = t378 ^ ((t378 ^ t379) & x5);
    let t381 = t376 ^ ((t376 ^ t380) & x2);
    let t382 = t372 ^ ((t372 ^ t381) & x1);
    let t383 = t11 ^ ((t11 ^ t12) & x7);
    let t384 = t56 ^ ((t56 ^ t383) & x3);
    let t385 = t59 ^ ((t59 ^ t48) & x3);
    let t386 = t384 ^ ((t384 ^ t385) & x5);
    let t387 = t13 & !x7;
    let t388 = t387 ^ ((t387 ^ t195) & x3);
    let t389 = t156 ^ ((t156 ^ t83) & x3);
    let t390 = t388 ^ ((t388 ^ t389) & x5);
    let t391 = t386 ^ ((t386 ^ t390) & x2);
    let t392 = t194 ^ ((t194 ^ t266) & x3);
    let t393 = t246 ^ ((t246 ^ t383) & x3);
    let t394 = t392 ^ ((t392 ^ t393) & x5);
    let t395 = t40 & x7;
    let t396 = t395 & !x3;
    let t397 = t31 ^ ((t31 ^ t27) & x7);
    let t398 = t396 ^ ((t396 ^ t397) & x5);
    let t399 = t394 ^ ((t394 ^ t398) & x2);
    let t400 = t391 ^ ((t391 ^ t399) & x1);
    let t401 = t382 ^ ((t382 ^ t400) & x0);
    [t67, t123, t174, t222, t271, t319, t360, t401]
}

/// Bitsliced `STREAM_SBOX[0]` — five A-register index bits in,
/// the `pqzyx` bits at positions 0 and 10 out.
///
/// Synthesis: ROBDD, order [1, 3, 0, 4, 2] — 20 gates.
#[inline]
fn stream_sbox_0(x: &[Word; 5]) -> [Word; 2] {
    let [x0, x1, x2, x3, x4] = *x;
    let t0 = ONES & x2;
    let t1 = t0 | !x4;
    let t2 = t0 | x4;
    let t3 = t1 ^ ((t1 ^ t2) & x0);
    let t4 = ONES & !x2;
    let t5 = t3 ^ ((t3 ^ t4) & x3);
    let t6 = t0 ^ ((t0 ^ t4) & x4);
    let t7 = t6 & !x0;
    let t8 = t4 ^ ((t4 ^ t0) & x4);
    let t9 = t8 ^ ((t8 ^ t6) & x0);
    let t10 = t7 ^ ((t7 ^ t9) & x3);
    let t11 = t5 ^ ((t5 ^ t10) & x1);
    let t12 = t6 & x0;
    let t13 = t1 ^ ((t1 ^ t0) & x0);
    let t14 = t12 ^ ((t12 ^ t13) & x3);
    let t15 = t8 | !x0;
    let t16 = t0 & x4;
    let t17 = t16 ^ ((t16 ^ t6) & x0);
    let t18 = t15 ^ ((t15 ^ t17) & x3);
    let t19 = t14 ^ ((t14 ^ t18) & x1);
    [t11, t19]
}

/// Bitsliced `STREAM_SBOX[1]` — five A-register index bits in,
/// the `pqzyx` bits at positions 1 and 11 out.
///
/// Synthesis: ROBDD, order [2, 4, 1, 0, 3] — 19 gates.
#[inline]
fn stream_sbox_1(x: &[Word; 5]) -> [Word; 2] {
    let [x0, x1, x2, x3, x4] = *x;
    let t0 = ONES & !x3;
    let t1 = ONES & x3;
    let t2 = t0 ^ ((t0 ^ t1) & x0);
    let t3 = t0 & !x0;
    let t4 = ONES & x0;
    let t5 = t3 ^ ((t3 ^ t4) & x1);
    let t6 = t2 ^ ((t2 ^ t5) & x4);
    let t7 = t0 ^ ((t0 ^ t2) & x1);
    let t8 = t0 | x0;
    let t9 = t1 ^ ((t1 ^ t0) & x0);
    let t10 = t8 ^ ((t8 ^ t9) & x1);
    let t11 = t7 ^ ((t7 ^ t10) & x4);
    let t12 = t6 ^ ((t6 ^ t11) & x2);
    let t13 = t0 & x0;
    let t14 = t13 | !x1;
    let t15 = t14 ^ ((t14 ^ t7) & x4);
    let t16 = t9 ^ ((t9 ^ t0) & x1);
    let t17 = t5 ^ ((t5 ^ t16) & x4);
    let t18 = t15 ^ ((t15 ^ t17) & x2);
    [t12, t18]
}

/// Bitsliced `STREAM_SBOX[2]` — five A-register index bits in,
/// the `pqzyx` bits at positions 2 and 4 out.
///
/// Synthesis: ROBDD, order [0, 1, 3, 2, 4] — 16 gates.
#[inline]
fn stream_sbox_2(x: &[Word; 5]) -> [Word; 2] {
    let [x0, x1, x2, x3, x4] = *x;
    let t0 = ONES & x4;
    let t1 = ONES & !x4;
    let t2 = t0 ^ ((t0 ^ t1) & x3);
    let t3 = ONES & x2;
    let t4 = ONES & !x2;
    let t5 = t3 ^ ((t3 ^ t4) & x3);
    let t6 = t2 ^ ((t2 ^ t5) & x0);
    let t7 = t1 | !x2;
    let t8 = t7 ^ ((t7 ^ t3) & x3);
    let t9 = t1 ^ ((t1 ^ t0) & x2);
    let t10 = t8 ^ ((t8 ^ t9) & x1);
    let t11 = t1 & !x2;
    let t12 = t3 ^ ((t3 ^ t11) & x3);
    let t13 = t4 ^ ((t4 ^ t9) & x3);
    let t14 = t12 ^ ((t12 ^ t13) & x1);
    let t15 = t10 ^ ((t10 ^ t14) & x0);
    [t6, t15]
}

/// Bitsliced `STREAM_SBOX[3]` — five A-register index bits in,
/// the `pqzyx` bits at positions 3 and 5 out.
///
/// Synthesis: ROBDD, order [4, 1, 2, 3, 0] — 17 gates.
#[inline]
fn stream_sbox_3(x: &[Word; 5]) -> [Word; 2] {
    let [x0, x1, x2, x3, x4] = *x;
    let t0 = ONES & !x0;
    let t1 = t0 | !x3;
    let t2 = ONES & x0;
    let t3 = t2 & x3;
    let t4 = t1 ^ ((t1 ^ t3) & x2);
    let t5 = t2 ^ ((t2 ^ t0) & x3);
    let t6 = t2 ^ ((t2 ^ t5) & x2);
    let t7 = t4 ^ ((t4 ^ t6) & x1);
    let t8 = t0 ^ ((t0 ^ t2) & x3);
    let t9 = ONES & !x3;
    let t10 = t9 ^ ((t9 ^ t0) & x2);
    let t11 = t8 ^ ((t8 ^ t10) & x1);
    let t12 = t7 ^ ((t7 ^ t11) & x4);
    let t13 = t3 ^ ((t3 ^ t1) & x2);
    let t14 = t0 ^ ((t0 ^ t8) & x2);
    let t15 = t13 ^ ((t13 ^ t14) & x1);
    let t16 = t11 ^ ((t11 ^ t15) & x4);
    [t12, t16]
}

/// Bitsliced `STREAM_SBOX[4]` — five A-register index bits in,
/// the `pqzyx` bits at positions 6 and 8 out.
///
/// Synthesis: ROBDD, order [0, 1, 3, 4, 2] — 22 gates.
#[inline]
fn stream_sbox_4(x: &[Word; 5]) -> [Word; 2] {
    let [x0, x1, x2, x3, x4] = *x;
    let t0 = ONES & x2;
    let t1 = t0 | x4;
    let t2 = ONES & !x2;
    let t3 = t2 & !x4;
    let t4 = t1 ^ ((t1 ^ t3) & x3);
    let t5 = t4 ^ ((t4 ^ t0) & x1);
    let t6 = t2 | !x4;
    let t7 = t6 & x3;
    let t8 = t2 ^ ((t2 ^ t0) & x4);
    let t9 = t2 | x4;
    let t10 = t8 ^ ((t8 ^ t9) & x3);
    let t11 = t7 ^ ((t7 ^ t10) & x1);
    let t12 = t5 ^ ((t5 ^ t11) & x0);
    let t13 = t0 & !x4;
    let t14 = t13 & !x3;
    let t15 = t14 | !x1;
    let t16 = t0 ^ ((t0 ^ t2) & x4);
    let t17 = t16 ^ ((t16 ^ t2) & x3);
    let t18 = t0 & x4;
    let t19 = t16 ^ ((t16 ^ t18) & x3);
    let t20 = t17 ^ ((t17 ^ t19) & x1);
    let t21 = t15 ^ ((t15 ^ t20) & x0);
    [t12, t21]
}

/// Bitsliced `STREAM_SBOX[5]` — five A-register index bits in,
/// the `pqzyx` bits at positions 7 and 9 out.
///
/// Synthesis: ANF — 37 gates.
#[inline]
fn stream_sbox_5(x: &[Word; 5]) -> [Word; 2] {
    let [x0, x1, x2, x3, x4] = *x;
    let t0 = x2 & x0;
    let t1 = x2 & x1;
    let t2 = x3 & x1;
    let t3 = x3 & x2;
    let t4 = t1 & x0;
    let t5 = t2 & x0;
    let t6 = t3 & x0;
    let t7 = t3 & x1;
    let t8 = x4 & x1;
    let t9 = t8 & x0;
    let t10 = x4 & x2;
    let t11 = t10 & x1;
    let t12 = x4 & x3;
    let t13 = t12 & x0;
    let t14 = t11 & x0;
    let t15 = t12 & x1;
    let t16 = t15 & x0;
    let t17 = t12 & x2;
    let t18 = t17 & x1;
    let t19 = x0 ^ x2;
    let t20 = t19 ^ t1;
    let t21 = t20 ^ t4;
    let t22 = t21 ^ t2;
    let t23 = t22 ^ t3;
    let t24 = t23 ^ t7;
    let t25 = t24 ^ t9;
    let t26 = t25 ^ t11;
    let t27 = t26 ^ t14;
    let t28 = t27 ^ t16;
    let t29 = t28 ^ t18;
    let t30 = x1 ^ t0;
    let t31 = t30 ^ t5;
    let t32 = t31 ^ t3;
    let t33 = t32 ^ t6;
    let t34 = t33 ^ x4;
    let t35 = t34 ^ t9;
    let t36 = t35 ^ t13;
    [t29, t36]
}

/// Bitsliced `STREAM_SBOX[6]` — five A-register index bits in,
/// the `pqzyx` bits at positions 12 and 13 out.
///
/// Synthesis: ROBDD, order [1, 2, 3, 4, 0] — 22 gates.
#[inline]
fn stream_sbox_6(x: &[Word; 5]) -> [Word; 2] {
    let [x0, x1, x2, x3, x4] = *x;
    let t0 = ONES & x4;
    let t1 = ONES & x0;
    let t2 = t1 | !x4;
    let t3 = t0 ^ ((t0 ^ t2) & x3);
    let t4 = t2 & !x3;
    let t5 = t3 ^ ((t3 ^ t4) & x2);
    let t6 = ONES & !x0;
    let t7 = t6 & !x4;
    let t8 = t7 ^ ((t7 ^ t0) & x3);
    let t9 = t1 & !x4;
    let t10 = t9 | x3;
    let t11 = t8 ^ ((t8 ^ t10) & x2);
    let t12 = t5 ^ ((t5 ^ t11) & x1);
    let t13 = t6 & x4;
    let t14 = t13 ^ ((t13 ^ t6) & x3);
    let t15 = t7 | !x3;
    let t16 = t14 ^ ((t14 ^ t15) & x2);
    let t17 = t1 | x4;
    let t18 = t17 ^ ((t17 ^ t2) & x3);
    let t19 = t1 & x3;
    let t20 = t18 ^ ((t18 ^ t19) & x2);
    let t21 = t16 ^ ((t16 ^ t20) & x1);
    [t12, t21]
}

/// Bitsliced `STREAM_CDEF[..]` — the C/D/E/F feedback table, ten index
/// bits in, its nine live output bits out (positions
/// [`CDEF_OUT_BITS`]); every other position of the table is zero.
///
/// Synthesis: ROBDD, order [9, 2, 7, 6, 3, 1, 5, 8, 4, 0] — 58 gates.
#[inline]
pub(super) fn stream_cdef(x: &[Word; 10]) -> [Word; 9] {
    let [x0, x1, x2, x3, x4, x5, x6, x7, x8, x9] = *x;
    let t0 = ONES & x0;
    let t1 = ONES & !x0;
    let t2 = t0 ^ ((t0 ^ t1) & x4);
    let t3 = ONES & x5;
    let t4 = ONES & !x5;
    let t5 = t3 ^ ((t3 ^ t4) & x1);
    let t6 = ONES & x6;
    let t7 = ONES & !x6;
    let t8 = t6 ^ ((t6 ^ t7) & x2);
    let t9 = ONES & x3;
    let t10 = ONES & !x3;
    let t11 = t9 ^ ((t9 ^ t10) & x7);
    let t12 = t1 ^ ((t1 ^ t0) & x4);
    let t13 = t2 ^ ((t2 ^ t12) & x8);
    let t14 = t0 ^ ((t0 ^ t13) & x9);
    let t15 = ONES & x1;
    let t16 = t0 & x4;
    let t17 = t0 | x4;
    let t18 = t16 ^ ((t16 ^ t17) & x8);
    let t19 = t1 | !x4;
    let t20 = t1 & !x4;
    let t21 = t19 ^ ((t19 ^ t20) & x8);
    let t22 = t18 ^ ((t18 ^ t21) & x5);
    let t23 = t21 ^ ((t21 ^ t18) & x5);
    let t24 = t22 ^ ((t22 ^ t23) & x1);
    let t25 = t15 ^ ((t15 ^ t24) & x9);
    let t26 = ONES & x2;
    let t27 = t18 & x5;
    let t28 = t18 | x5;
    let t29 = t27 ^ ((t27 ^ t28) & x1);
    let t30 = t21 | !x5;
    let t31 = t21 & !x5;
    let t32 = t30 ^ ((t30 ^ t31) & x1);
    let t33 = t29 ^ ((t29 ^ t32) & x6);
    let t34 = t32 ^ ((t32 ^ t29) & x6);
    let t35 = t33 ^ ((t33 ^ t34) & x2);
    let t36 = t26 ^ ((t26 ^ t35) & x9);
    let t37 = t29 ^ ((t29 ^ t32) & x3);
    let t38 = t9 ^ ((t9 ^ t37) & x6);
    let t39 = t32 ^ ((t32 ^ t29) & x3);
    let t40 = t10 ^ ((t10 ^ t39) & x6);
    let t41 = t38 ^ ((t38 ^ t40) & x7);
    let t42 = t37 ^ ((t37 ^ t10) & x6);
    let t43 = t39 ^ ((t39 ^ t9) & x6);
    let t44 = t42 ^ ((t42 ^ t43) & x7);
    let t45 = t41 ^ ((t41 ^ t44) & x2);
    let t46 = t9 ^ ((t9 ^ t45) & x9);
    let t47 = ONES & x8;
    let t48 = t29 & x3;
    let t49 = t48 & x6;
    let t50 = t29 | x3;
    let t51 = t9 ^ ((t9 ^ t50) & x6);
    let t52 = t49 ^ ((t49 ^ t51) & x7);
    let t53 = t48 ^ ((t48 ^ t9) & x6);
    let t54 = t50 | x6;
    let t55 = t53 ^ ((t53 ^ t54) & x7);
    let t56 = t52 ^ ((t52 ^ t55) & x2);
    let t57 = t47 ^ ((t47 ^ t56) & x9);
    [t2, t5, t8, t11, t14, t25, t36, t46, t57]
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
