# §4 Experiments (section-level deep summary)

## §4.1 Setup

- **Benchmarks**: OfficeQA, SealQA, ALFWorld — chosen to "span complementary capabilities" (two QA, one embodied planning).
- **Splits**: within the evolution loop each benchmark is split by stratified sampling into train (failure mining) / val (child scoring + best-skill selection) / **held-out test** (never observed by loop). Final reporting is on the held-out test partition through a separate benchmark-mode pass.
- **Backbone**: single frozen Gemma-4 31B (Google, 2026), shared across all five pipeline agents. No fine-tuning.

**Baselines**: No-Skill (raw backbone), Static Skill (hand-authored `s₀` held fixed), Single-Level Evolution (fast loop with slow loop frozen: `K_max=1`, no cross-branch sharing, no meta-updates), full MetaSkill-Evolve.

**What I notice:**
- The "Single-Level Evolution" baseline is an internal ablation — it isolates whether meta-skill updates matter *given* task-skill evolution is already on. This is the most epistemically important comparison.
- No external prior-work baselines (EvoSkill, GEPA, SkillWeaver) are compared *empirically* here — they were named in §1 as instances of the analyze→propose→evolve pattern but the empirical comparison is against a self-implemented reduction (Single-Level Evolution). This is a limitation: we know the paper beats *its own reduction*, not necessarily *the published state of the art*.
- Frozen backbone shared across all five agents is a strong control. All five agents *are the same LLM* prompted differently. Any capability gain must live in the Markdown files.

---

## §4.2 Main Results

| Method | OfficeQA | SealQA | ALFWorld |
|---|---:|---:|---:|
| No-Skill | 31.78 | 29.17 | 92.31 |
| Static skill | 36.09 | 29.41 | 90.38 |
| Single-Level | 48.94 | 37.21 | 92.31 |
| **MetaSkill-Evolve** | **55.32** | **45.26** | **94.23** |
| Δ vs. No-Skill | +23.54 | +16.09 | +1.92 |

**The paper's three-part argument:**
1. Static skill > No-Skill on OfficeQA (+4.31), roughly neutral on SealQA (+0.24), *regresses* on ALFWorld (−1.93).
2. Single-Level > Static on QA (+12.85 / +7.80), no gain on ALFWorld.
3. Full > Single-Level: **+6.38 / +8.05 / +1.92**. This is the paper's meta-loop attribution.

**What I notice:**
- The QA monotonicity story is real and clean: each step adds points on both QA benchmarks. This is strong evidence that each design choice matters — at least in the QA regime.
- **ALFWorld tells a different story**. Static regresses. Single-Level does not recover. Only the meta-loop supplies any gain, and it is only +1.92. The paper flips this: "meta-skill adaptation remains the operative ingredient even once task-skill evolution has saturated." But the more sceptical reading is that ALFWorld is near ceiling (92.31%) and 1.92 points could plausibly be noise-level. There is no confidence interval reported here.
- The Δ vs. No-Skill headline (+23.54 etc.) is the number in the abstract and is genuinely large on the QA tasks. But the *slow-loop attribution* is +6.38 / +8.05 — real, but a much more modest headline. The abstract chooses the bigger number, which is fair but blurs the attribution.
- No variance reported. No seeds, no confidence intervals. This is the biggest empirical weakness.

---

## §4.3 Component Ablations

Disable one meta-skill component at a time. `−ψ`, `−σ`, `−α`, `−π` remove the corresponding component. Two extra conditions:
- `−σ_x`: remove *only cross-branch* candidates (same-branch inspirations preserved).
- **No meta-updates**: freeze the slow loop entirely.

`ε` never gets its own ablation row because it always executes (with `−π` it consumes raw analysis instead of a structured proposal).

**Three headline findings:**
1. **Every component contributes** — no ablation matches the full system.
2. **OfficeQA: `α` dominates**. Removing `α`: 55.32 → 35.58 (−19.7). "The OfficeQA failure landscape contains pockets of related arithmetic errors where `α`'s adaptive widening of the child budget after stagnation is what produces a successful child at all." `π` is a close second (−17.7).
3. **SealQA: `π` dominates**. 45.26 → 36.84 with `π` removed. "Gain hinges on the precise content of each edit rather than on how widely the search fans out."

**What I notice:**
- The story about domain-specific dominance is plausible and lines up with intuition: arithmetic-heavy tasks (OfficeQA) benefit from budget widening because a lot of adjacent proposals succeed; harder single-shot reasoning (SealQA) benefits from proposal quality because the answer space is not a neighbourhood you can cover by fan-out.
- The `ε` non-ablation is a small honesty gap. The paper argues `ε` "always executes" so it cannot be turned off. But `ε` could still be *frozen* to a hand-authored initial version while others evolve, and the paper does not report that. Similarly there is no all-evolve-except-`ε` condition.
- No meta-updates is the *most important* ablation and is folded into the main table as "Single-Level" — so §4.3 is doing component-level attribution *within* the full system, not questioning whether the whole meta-loop matters (that is §4.2's job).

---

## §4.4 Meta-Update Horizon

Sweep `H ∈ {2, 4, 8}` with meta-update count held at **3**, so total iterations `= 3H = {6, 12, 24}`.

**Result:** `H=2` best on every benchmark. OfficeQA most sensitive (−9.1 pts from H=2 to H=8). SealQA and ALFWorld nearly flat between H=2 and H=4, small drops at H=8.

**Argument:** all three horizons aggregate over multiple iterations (escaping per-iteration noise); among them "the most reactive schedule wins."

**What I notice:**
- The compute-normalisation is subtle: fixing the meta-update count at 3 (rather than fixing total iterations) means H=2 gets *fewer* fast iterations (6) than H=8 (24). So H=2 winning is *despite* fewer fast iterations, which strengthens the "reactive is better" claim.
- But this also means H=8 has 4× the raw fast-loop compute of H=2 and *still loses*. That is genuinely informative.
- Not swept: `H=1` (per-iteration meta-updates). The paper explicitly argued in §3.5 that this would expose meta-skills to single-example noise, and does not test it. A reader might wonder if the argument is empirically confirmed.
- The paper picks H=2 as default. In Tables 1 and 3 the setup uses *five iterations* with *two* meta-updates — so "H=2" in the main table is a slightly different operating point than "H=2" in the sweep (which has three meta-updates and six iterations). The paper flags this honestly.

---

## §5 Conclusion

Restates the framing:
- Every branch carries `s` and `m = (ψ,σ,α,π,ε)`.
- Meta-skill components are Markdown files → same pipeline refines them.
- Frontier score `η₁U_v + η₂P̂_v + η₃N_v` redirects search from plateaued branches.
- Results: +23.54 / +16.09 / +1.92 over raw backbone; slow-loop contribution +6.38 / +8.05 / +1.92.
- Punchline: "an agent's improvement policy admits the same search machinery as its task behaviour, and … separating what to do from how to improve keeps each loop's signal interpretable."

**What I notice:**
- The final claim is not just an empirical one — it is an *epistemological* one about interpretability. The paper is arguing that separating fast and slow loops makes attribution *easier*. This is a genuine benefit distinct from performance: you can point at which loop moved the needle.
- The bounded one-level recursion is not defended against a two-level extension. The natural question — why not evolve the meta-meta-skill? — is not addressed. Implicitly, the answer is "compute" and "diminishing returns," but neither is stated.
