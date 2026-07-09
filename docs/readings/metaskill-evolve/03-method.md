# §3 MetaSkill-Evolve (paragraph-level, downshifting to sentence in §3.2)

## §3.1 Problem Formulation

$$U(s) = \mathbb{E}_{(x,y)\sim\mathcal{T}}[r(A_s(x), y)]$$

- `s` is a Markdown-format LLM-agent program.
- Reward `r(·,·) ∈ [0,1]`.
- `U(s)` estimated as validation-batch accuracy.

**What I notice:** the formalisation is minimal and standard — it exists to license `U(s)` as a proper objective. Nothing controversial.

---

## §3.2 Branch State and Meta-Skill — sentence-level

This section is the conceptual core, so I downshift.

> "Each task-skill iteration turns a failure into a skill edit through a fixed five-step procedure, i.e., diagnose, retrieve, allocate, propose, execute."

The paper *asserts* that the pipeline has five steps. The five-step framing is not derived from prior literature — it is the framing this paper is proposing. Diagnose→propose→execute is standard; retrieve and allocate are the additions.

> "MetaSkill-Evolve makes that procedure adaptive by attaching to each branch a meta-skill `m` that parameterises all five steps."

*Attaching to each branch* is the key move. The meta-skill is **branch-local**, not global. This is a design choice with real consequences — different lineages can develop different improvement policies. It also means cross-branch retrieval (`σ`) is the only channel through which one lineage's meta-innovations reach another.

> "A branch state is `b = (s, m, h)`."

`h` finally gets defined implicitly: "the branch's iteration history." It will show up as input to the Allocator and as the raw material for the meta-failure trace.

$$m = (\psi, \sigma, \alpha, \pi, \varepsilon)$$

The five components, one per specialist agent:

- **ψ – Analyzer** — diagnosis policy; maps failures to a tag `φ` + free-form analysis `a`.
- **σ – Retriever** — sharing policy; selects same-branch and cross-branch inspirations matching `φ`.
- **α – Allocator** — allocation policy; sets child budget `K ∈ [1, K_max]`.
- **π – Proposer** — edit-proposal policy; emits edit `δ` conditioned on `(f, a, ℐ)`.
- **ε – Evolver** — edit-executor policy; writes `δ` to disk and verifies.

**Sentence-level notes on the tuple:**
- `ψ` returns *both* a discrete tag and a free-form analysis. The tag drives retrieval; the analysis feeds the Proposer. The **tag vocabulary itself is maintained by `ψ`** (revealed later in §3.4). This is a recursive move inside the fast loop already — the tag ontology is not fixed.
- `σ` retrieves from same-branch *and* cross-branch. Cross-branch retrieval is the primary lateral communication channel in the whole system.
- `α` bounds the search fan-out to `K_max`. The paper does not say what `K_max` is here.
- `π` conditions on `(f, a, ℐ)` — worst case, analysis, inspirations. Notably it does *not* condition on the branch history `h`; that goes into `α`.
- `ε` is described as "writes and verifies" — the verification step is a before/after hash check (from §3.4). Trivial-looking but load-bearing: it flags null edits that leave files unchanged, which is the failure mode where the Proposer produces something the Evolver cannot actually apply.

> "Since each meta-skill file uses the same Markdown representation as the task-skill files the pipeline already consumes, the same five agents that improve `s` also improve `m` when applied recursively."

The representational trick, restated. This is what makes the recursion cheap.

$$P(m \mid s) = \mathbb{E}\left[\frac{1}{K}\sum_{k=1}^{K}\bigl(U(s'_k) - U(s)\bigr)\right]$$

Estimated per-node as `P̂_v = mean over children of ΔU`. **Zero for nodes with no children.**

**What I notice at the sentence level:**
- Meta-productivity is a *conditional* expectation `P(m|s)` but is estimated as an *empirical mean over actual children*. This means the estimate is confounded — you cannot separate "this `m` is productive" from "this `s` was easy to improve." The paper acknowledges the joint objective but does not disentangle attribution.
- Setting `P̂_v = 0` for childless nodes is not neutral. It means an unexplored branch has the *same* meta-productivity signal as a branch that produced only failed children. The novelty term `N_v` is what rescues unexplored branches from being punished by this.
- **Concept update:** the two objectives `U` and `P` are not merely different quantities — they are *estimated on different populations*. `U(s)` is estimated on the validation batch (external data). `P̂_v` is estimated on descendants (internal search history). This asymmetry matters: `P̂_v` is much noisier and much more sample-limited.

---

## §3.3 Evolution Graph and Frontier Selection — paragraph level

### ¶ "We record the entire search history…"

**What it does:** Sets up the persistence structure — a DAG `𝒢 = (𝒱, 𝓔)` in SQLite. Nodes carry `(s_v, m_v, U_v, ΔU_v, φ_v)` plus branch path + selection counter. Two edge types:
- **Lineage edges** (parent → child).
- **Inspiration edges** (cross-branch source retrieved by `σ` when proposing).

**What I notice:**
- The DAG is *the ground truth*, not the working directory. Files on disk are reconstructed from snapshots when a branch is selected. This is essential for lineage isolation (prevents leakage) and for provenance.
- Recording inspiration edges is a distinctive choice — it means cross-branch influence is *auditable* per node. This is nice for later attribution work.
- Both edge types are directed forward in time; DAG is acyclic by construction. Nodes are never revised in place. This gives the graph *event-log* semantics rather than *state* semantics.

### ¶ "A child enters the archive…"

Only strict improvers (`ΔU_v > 0`) become future parents. Neutral/regressing children are still *persisted*, though — they remain available as inspiration for `σ`.

**What I notice:**
- This is a soft form of the classical archive filter (only accept improvements). But the paper *keeps* the rejected children in the graph as inspiration material. That means a failed edit can still teach a later branch what *not* to do — the failure remains visible to `σ`.
- Strict `> 0` (not `≥`) means accuracy-neutral edits do not propagate. This might over-filter in noisy regimes but keeps the archive clean.

### ¶ Frontier score

$$v^* = \arg\max_{v \in \mathcal{F}}\bigl[\eta_1 U_v + \eta_2 \hat{P}_v + \eta_3 N_v\bigr]$$

with `N_v = 1/(1 + times_selected_v)`.

The paper is explicit about what each term prevents:
- Without `U`: trusts noisy single-child gains.
- Without `P̂`: locks on stagnated high-utility.
- Without `N`: collapses to one lineage.

**What I notice:**
- The three terms map to a classic **quality / progress / diversity** decomposition. The paper is essentially importing a **MAP-Elites / quality-diversity** intuition where `P̂` acts as a *behavioural descriptor* (as they say in Related Work). This is elegant: rather than partition the search space into behavioural cells, they use meta-productivity itself as the diversity coordinate.
- The novelty term is *visitation-based* (how many times this node has been selected as parent) not *behavioural* (how different this node's `m` is from others). This is a cheaper choice but loses the sense in which two branches might be *behaviourally similar despite being visited differently*.
- **Crucially:** "we do not filter by lineage: diversity is a property of the score, not a structural constraint." This is a deliberate departure from beam-search / island-model traditions. The paper is claiming that persisting the full DAG plus a score-based diversity term subsumes structural diversity constraints.

---

## §3.4 Fast Timescale: Task-Skill Evolution — paragraph level

**Setup:** on selected parent `v`, restore `(s_v, m_v)` snapshots to disk *first*. Score `s_v` on training batch, take worst example as diagnostic target.

Then run the five agents:
1. **Analyzer** emits `(φ, a)`; tag vocabulary maintained by `ψ`, *revised by the slow loop.*
2. **Retriever** over-fetches 3× the inspiration budget by tag similarity, then LLM-reranks down. *Breadth/depth balance is itself a learned object.*
3. **Allocator** sets `K` — widens after stagnation, contracts after productive edit.
4. **Proposer** emits `K` edits; with `K>1` a diversity hint steers the k-th proposer to a distinct angle.
5. **Evolver** writes via `skill_tools` and does a before/after hash check.

Then every `H` iterations, slow loop refreshes `m_v`; the `K` children commit with the *refreshed* `m_v`.

**What I notice:**
- **Worst-case as diagnostic target** is a high-signal / high-variance choice. The paper justifies it by saying the edit is judged on `ΔU_v` (validation gain), not on the training case — so noise on the single case does not directly cost you. But there is still a selection bias: if the worst case is atypical, edits will be steered toward fixing atypical failures. This is not discussed.
- **"Tag vocabulary is itself maintained by `ψ`"** — this is a hidden recursion inside the fast loop. `ψ` is a Markdown file that includes the tag vocabulary; when the slow loop rewrites `ψ`, the tags change; downstream, `σ`'s retrieval and `π`'s conditioning both shift. This is a *coupled* update.
- **3× over-fetch then LLM re-rank** is a classic retrieval pattern (approximate then rerank). But the paper says the balance is "itself a learned object" — meaning the 3× or the reranking prompt could be modified when `σ` is rewritten. This is a nice concrete example of what "meta-skill evolution" actually changes.
- **`α` widens after stagnation, contracts after productive edit.** This is a UCB-flavoured schedule and is a policy that could plausibly be *bad* — over-widening on a genuine plateau, or over-contracting after a lucky child. The ablation showing `α` is dominant on OfficeQA (§4.3) suggests the initial `α` was already reasonable but the *evolved* `α` was materially better.
- **Diversity hint on `k`-th proposer** — this is a cheap trick against near-duplicate children when `K>1`, and it presumably lives in the `π` skill file itself, so it too evolves.
- **Refreshed-`m` at commit time**: children commit with the *new* `m_v`, not the one that produced them. That is subtle: the meta-skill that generated `s'_k` is not the meta-skill recorded on `s'_k`'s node. This is a *forward* attribution: the recorded `m` is the one you'd use *from* this node.

---

## §3.5 Slow Timescale: Meta-Skill Evolution — paragraph level

### ¶ Motivation

> "Updating `m` at every iteration would expose the meta-skill to the same single-example noise that drives task-skill evolution."

The slow loop fires once per `H` iterations, aggregates over that horizon: `P̂(m|s) = mean of ΔU_u over last H descendants`. This becomes a *synthetic meta-failure trace* `f_m` — "the improvement history reshaped to look like one failing training example." A single Analyzer prompt then serves both timescales.

**What I notice:**
- The **synthetic failure trace** is the key engineering move. To reuse the pipeline unchanged, the slow loop must supply an input in the pipeline's expected shape. So it *fakes* a failing training example out of a bundle of `H` past children.
- This is where the "same pipeline" claim gets a little bent. The pipeline is the same, but the *input* to it is a fabricated aggregate object. That is fine — but it means the pipeline is doing something categorically different (regressing an improvement policy) with the same interface.

### ¶ Three details separating slow from fast

1. **Constrained Analyzer** — must diagnose *one of `{ψ,σ,α,π,ε}`*. Falls back to round-robin on null / task-skill diagnoses. Prevents silent degradation.
2. **Whole-`m` rewrite** — every child rewrites *all five* meta-files in one step (Proposer sequential, Evolvers parallel). Preserves cross-component coherence.
3. **Accumulating children** — child `k+1` reads files as written by child `k`, *not* the parent. Moving target → incremental refinement rather than `K_m` overwrites averaging back.

**What I notice:**
- Detail 1 admits that the Analyzer can misfire on meta-inputs — the fallback exists because the diagnosis is not always trustworthy at the meta level. Round-robin is a graceful degradation.
- Detail 2 is important: the paper is not proposing to evolve one meta-component at a time. A `π` change that assumes finer tags is applied *together with* the matching `ψ` change. This handles coupling between components. But it also means slow-loop children are large diffs — the search is coarse-grained.
- Detail 3 (accumulating children) means the `K_m` proposals are *not* independent samples. Child 3 has seen edits 1 and 2. This makes the search *depth-first-ish inside a slow-loop round*, which is quite different from the fast loop's `K` parallel children.

### ¶ Cross-lineage propagation

> "Each branch therefore carries its own lineage-local `m`, and the sole channel by which one lineage's improvement policy reaches another is this meta-level retrieval, so escape strategies propagate between lineages without any shared global state."

**What I notice:** this is the paper reiterating that there is no global meta-skill — only per-lineage `m`s. Cross-branch `σ` retrieval is the sole propagation channel. This is a strong isolation guarantee. It also means the system can hold *contradictory* meta-skills in different lineages simultaneously without conflict — the DAG is genuinely a search over policies as well as skills.

---

## What sticks after §3

- The pipeline's five components map 1:1 to five things prior systems freeze — this is engineered, not discovered.
- The DAG is the ground truth; the working directory is derivative. Snapshot-restore isolation is what makes multiple lineages coherent.
- **Meta-productivity is estimated as empirical child-improvement mean, and is zero for childless nodes.** This is why the novelty term `N_v` exists.
- The frontier score decomposes into **exploitation / progress / diversity** with `P̂` acting as a quality-diversity descriptor.
- The slow loop is *not* structurally identical to the fast loop despite the "same pipeline" framing:
  - Its input is a fabricated aggregate (synthetic failure trace).
  - Its Analyzer is constrained to `{ψ,σ,α,π,ε}` with a round-robin fallback.
  - Its children are accumulating (depth-first within a round) rather than parallel.
  - Its Evolver rewrites all five files, not one.
- The recursion is one-level and lineage-local. There is no meta-meta-skill. Cross-lineage flow is entirely through `σ` retrieval.

The load-bearing claim of the whole method: the *representational identity* between meta-skills and task skills (both Markdown files) makes it *possible* to reapply the pipeline; but making the reapplication actually *productive* required four bespoke slow-loop modifications (synthetic trace, constrained analyzer, whole-`m` rewrite, accumulating children). The abstract's "no additional model or objective" is fair; the introduction's "same pipeline applied to itself" is somewhat overstated.
