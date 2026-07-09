# Running summary — MetaSkill-Evolve

Close reading of Wang et al., *MetaSkill-Evolve: Recursive Self-Improvement of LLM Agents via Two-Timescale Meta-Skill Evolution* (arXiv:2607.05297v1, 2026-07-06).

Per-section files:
- [[01-abstract-and-figure1]] — framing and the four-regimes figure
- [[02-introduction]] — critique of prior fixed-meta systems, contributions
- [[03-method]] — §3, the conceptual core (sentence-level in §3.2)
- [[04-experiments]] — §4 + Conclusion (section-level summary)

---

## Compressed narrative

The paper's argument in one arc:

1. Skills-as-Markdown-files are already the standard way to extend LLM agents. Existing self-improvement systems close a loop that *rewrites the skill* but keeps the rewriting procedure fixed.
2. Fixed-meta evolution stalls because it optimises only utility `U(s)`, ignoring a second quantity — *meta-productivity* `P(m|s)`, the rate at which a branch's improvement policy generates stronger children.
3. Fix: give each branch a **meta-skill** `m = (ψ, σ, α, π, ε)` that parameterises the pipeline. Because `m`'s components are themselves Markdown files, the same five-agent pipeline that rewrites `s` can be reapplied to `m`.
4. Two timescales: `s` evolves every iteration; `m` evolves every `H` iterations. The slow loop's driving signal is empirical meta-productivity over the last `H` descendants, packaged as a synthetic failure trace so the same pipeline interface applies.
5. A DAG persisted in SQLite holds the whole search history; frontier selection scores parents by `η₁U + η₂P̂ + η₃N` (utility / progress / diversity).
6. Empirically: +23.54 / +16.09 / +1.92 pts over raw Gemma-4 31B on OfficeQA / SealQA / ALFWorld. Attributable to the slow loop specifically: +6.38 / +8.05 / +1.92. Component ablations show domain-dependent dominance (`α` for OfficeQA, `π` for SealQA).

---

## The paper's central vocabulary and its shifts

| Term | First introduced | What it does |
|---|---|---|
| **skill** | ¶1 intro | Editable Markdown spec of reusable procedures; *file-system artifact*. |
| **task skill `s`** | §3.1 | The skill executed to do the task. |
| **meta-skill `m`** | §3.2 | Five-tuple parameterising the improvement pipeline. |
| **branch** | §3.2 | Lineage carrying `(s, m, h)`. Meta-skill is *branch-local*. |
| **utility `U(s)`** | §3.1 | Validation accuracy of `s`. |
| **meta-productivity `P(m|s)`** | ¶3 intro (concept), §3.2 (formula) | Expected per-child improvement; estimated as empirical child-Δ mean; zero for childless nodes. |
| **frontier score** | ¶6 intro (preview), §3.3 (defined) | `η₁U + η₂P̂ + η₃N`. Exploitation / progress / diversity. |
| **fast / slow timescale** | Abstract, §3.5 | `s` every iteration, `m` every `H`. |
| **recursively self-improving** | §1 (Good, Schmidhuber lineage) | Improving the operator, not just the operand. Paper claims *bounded one-level*. |
| **synthetic meta-failure trace** | §3.5 | Aggregated `H`-window history reshaped to look like a single failing training example. Bridges the interface between fast and slow loops. |

**Key vocabulary tension:** the paper repeatedly says "the same pipeline is applied to itself." But at the slow-loop level, four things differ (§3.5): synthetic-trace input, constrained Analyzer with round-robin fallback, whole-`m` rewrites, and accumulating children. "Same pipeline" is honest at the *implementation* level (same five agents, same file format) but overstated at the *dynamics* level.

---

## Emerging patterns across the paper

### The engineered five-tuple

The five meta-skill components (`ψ, σ, α, π, ε`) are *not* discovered by the paper — they are engineered to match the five things prior systems freeze. The critique in §1¶2 lists five fixed things; §3.2 introduces exactly five meta-skill components; §4.3 ablates exactly those five. This is fine, but it means the tuple's completeness is asserted rather than derived. A reader could reasonably ask: are there other things prior systems also freeze but that this paper's tuple does not capture?

### Meta-productivity as quality-diversity descriptor

The frontier score is genuinely elegant. Rather than partition the search space behaviourally (as MAP-Elites would), it uses `P̂` — the *rate of improvement* — as the descriptor. This gets diversity "for free" as a property of the score, so no lineage-structural constraints are needed. §3.3 makes this explicit.

### The DAG as ground truth

Working-directory state is derivative. Every branch selection restores its recorded snapshot before running. The DAG (SQLite) is the actual system state. This buys clean isolation between lineages and full provenance — but it also means the system's memory grows unboundedly with search history. The paper does not discuss archival policy.

### Compute claims

"No additional model" is honest. "No additional objective" is somewhat massaged — `P(m|s)` is a new quantity but it is composed into a single frontier score with `U`. "Same pipeline" is honest at the interface level, overstated at the dynamics level.

---

## What is missing / where I would push

- **No comparison to published prior systems.** The paper names EvoSkill / GEPA / SkillWeaver but does not benchmark against them, only against internal reductions. Single-Level Evolution is a *reduction of MetaSkill-Evolve*, not the same as a faithful re-implementation of prior work.
- **No variance / seeds.** All numbers are point estimates. On ALFWorld the reported +1.92 gain has no error bar and the static skill's −1.93 regression *from* baseline is the same order of magnitude — a coincidence worth flagging.
- **`ε` is never ablated.** The paper's justification (`ε` "always executes") does not preclude freezing `ε` while others evolve. This leaves the executor's contribution untested.
- **`H=1` is not swept.** The argument against per-iteration meta-updates is intuitive but empirically untested; the sweep starts at H=2.
- **Two-level recursion is not addressed.** Why stop at `m`? The paper calls it "bounded, one-level recursion" but does not explore or motivate the bound. A meta-meta-skill would follow the same recipe.
- **Cold-start on `P̂`.** Childless nodes get `P̂ = 0`. The novelty term rescues them at selection time, but for early iterations the frontier score is dominated by `U` and `N` (since almost no node has children yet). The dynamics of the first few iterations are not analysed.

---

## What surprised

- The **representational identity trick** (meta-skills as Markdown files, same as task skills) is doing more work than the "same pipeline" phrasing suggests. It is what makes the recursion *cheap* and *implementable*. Without it, the whole framework would need a separate meta-optimiser.
- The **synthetic failure trace** is a modest but load-bearing engineering move: it lets the Analyzer prompt serve both loops. Everything downstream of the Analyzer benefits from this reuse.
- **Meta-productivity as a *quality-diversity descriptor*** rather than merely a secondary objective. Once you see it as a QD descriptor, the frontier score's shape stops feeling ad-hoc.
- **On OfficeQA, `α` is more important than `π`.** I would have guessed proposal quality dominates everywhere; it does not. The Allocator's fan-out schedule turns out to be the highest-leverage single component on a benchmark whose failure landscape is neighbourhood-structured.

---

## What sticks (final)

Two things I want to carry forward:

1. **When an artifact and its optimiser share a representation, recursion becomes cheap.** The Markdown-file identity between `s` and `m` is not a technical detail — it is the design pivot that makes the whole framework possible. Any similar system that wants to be recursively self-improving should ask: does my optimiser have the same representation as its operand?

2. **Meta-productivity is a useful search coordinate even outside recursive self-improvement.** Scoring branches by *the rate at which they generate improvements* — not by their current best — is a general move that would probably help any evolutionary search where children can be evaluated. The paper's presentation of it as a QD descriptor is the framing to steal.

Related to river-engine's own concerns (witness, memory, skills-as-files): the DAG-as-ground-truth pattern here has the same shape as our workspace/session persistence — the working tree is derivative, the persisted state is authoritative. And the branch-local meta-skill idea maps interestingly onto per-lineage witness/memory state.
