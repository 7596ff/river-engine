# §1 Introduction (paragraph-level argumentative)

## ¶1 — "Language model agents now tackle…"

**What this paragraph does:**
- Frames skills as a "productive remedy" to the fact that agents "rarely succeed out of the box."
- Names three prior systems (EvoSkill, GEPA, SkillWeaver) as instances of the "analyze–propose–evolve" pipeline.
- Sets up the space it will critique.

**Key phrase:** *"a curated, editable Markdown specification of reusable procedures, now a portable file-system artifact in widely deployed agent harnesses."*

**What I notice:** the paper is at pains to establish that skills-as-files is *already the standard* — not a strange choice they are making. This lets them present their contribution as merely extending an existing artifact type, not inventing one.

---

## ¶2 — "These systems, however, evolve only what the agent does…"

**What this paragraph does:**
- Sharpens the critique from ¶1 into a slogan: *"what the agent does, not how it evolves."*
- Invokes Good (1965) and Schmidhuber (2006) to place the critique in the lineage of recursive-self-improvement discourse.
- Enumerates what is fixed in prior systems: diagnosis style, edit type, search budget, cross-branch sharing, disk-write procedure.
- Closes with a concrete failure case: a branch cannot revise its diagnosis procedure when it repeatedly misfires.

**Key distinction:** *self-improving* vs. *recursively self-improving* (from Good/Schmidhuber's vocabulary). The paper is claiming prior work is only the first.

**What I notice:**
- The enumeration of five fixed things — "how failures are diagnosed, which edits are proposed, how much search effort is allocated, whether cross-branch experience is reused, and how an approved edit is applied to disk" — is *exactly* the five components of `m = (ψ, σ, α, π, ε)` that will be introduced later. This is a rhetorical setup: the argument in ¶2 licenses the exact five-tuple in ¶4. The tuple is not derived; it is anticipated by the critique.

---

## ¶3 — "A closer look at this rigidity…"

**What this paragraph does:**
- Distinguishes two quantities: **utility** `U(s)` and **meta-productivity** `P(m|s)`.
- Argues these are independent: high-utility skill may sit in a slow-improving branch; moderate skill may sit in a fast-improving one.
- Diagnoses fixed-meta evolution as failing precisely because it ignores `P(m|s)`.

**Concept construction:**
- `U(s)` = "score of the present skill on a validation batch."
- `P(m|s)` = "the rate at which a branch generates stronger descendants under its current improvement policy `m`."

**What I notice:**
- `P(m|s)` is being introduced *conceptually* here, before it is defined mathematically in §3.2. That is deliberate: the paper wants the reader to accept the *intuition* first.
- The example (high-U skill in a low-P branch vs. low-U skill in a high-P branch) is doing the work of an existence proof by analogy — it makes the two quantities feel independent without needing to demonstrate independence formally.
- Hidden assumption: that meta-productivity is *estimable* per branch at all. This will become an empirical mean over children later; the paper does not yet flag that a childless branch has no signal.

---

## ¶4 — Research Question box

> "Can the improvement procedure itself be evolved as a first-class object alongside the task skills it produces, using the same agentic pipeline?"

**What I notice:** the question is engineered — it is phrased so the answer is the paper's system. Two things are packed in:
1. "first-class object" — code for "same type as the task skill" (Markdown file).
2. "same agentic pipeline" — code for "no new machinery."

These are the paper's two cost-savings claims dressed as neutral phrasing.

---

## ¶5 — "To this end, we introduce MetaSkill-Evolve…"

**What this paragraph does:**
- Defines branch state `b = (s, m, h)`.
- Introduces the five-tuple `m = (ψ, σ, α, π, ε)` with one-line jobs each.
- States the fast/slow rhythm: `s` every iteration, `m` every `H` iterations.
- Says the slow loop is *driven by* "how much the branch's last `H` descendants improved" — i.e., meta-productivity is the training signal for meta-skill evolution.

**Key distinctions:**
- Each component is a *policy*: diagnosis policy, sharing policy, allocation policy, edit-proposal policy, edit-executor policy.
- `h` (iteration history) is quietly introduced as part of branch state, though its role is not yet clear — it will feed the Allocator and the slow loop's meta-failure trace.

---

## ¶6 — "Crucially, this adds no architectural component…"

**What this paragraph does:**
- States the core representational trick: each meta-skill component is a Markdown file identical in format to a task skill.
- Concludes: the *same five agents* rewrite `m` as rewrite `s`.
- Introduces the frontier score `η₁U(s) + η₂P(m|s) + η₃N(b)`, where `N(b)` is a novelty term.

**What I notice:**
- The word "Crucially" flags this as the load-bearing paragraph. The paper's cost claim rests entirely on the format-identity of meta-skills and task skills.
- The frontier score is introduced *here*, before the graph structure exists to make sense of it. This is a preview — the reader is told the shape of the objective early so the later mechanics feel motivated.
- The novelty term `N(b)` "discounts branches already selected many times" — this is a soft diversity mechanism, not a lineage constraint. Watch for the paper reiterating: *"we do not filter by lineage: diversity is a property of the score, not a structural constraint."*

---

## ¶7 — Results preview

- Frozen Gemma-4 31B backbone shared across all five pipeline agents.
- +23.54 / +16.09 / +1.92 vs. No-Skill; +6.38 / +8.05 / +1.92 vs. Single-Level Evolution on OfficeQA / SealQA / ALFWorld.
- Progression is monotonic on QA benchmarks; ALFWorld near ceiling so margins are small.

**What I notice:**
- The ALFWorld gain is very small (1.92) and the static skill *regresses* relative to No-Skill (−1.93). This is quietly acknowledged. The paper handles it by saying the backbone is near ceiling. But this also means the ALFWorld number is the *weakest* evidence for meta-skill evolution mattering — the noise floor is high.
- The frame "of which the slow loop contributes X" is careful. The paper is claiming *incremental* attribution: adding a skill helps, evolving it helps *further*, evolving the evolver helps *further still*. The monotonicity is presented as the strongest evidence.

---

## ¶8 — Contributions

Four numbered contributions restating the paper's claims:
1. Two-timescale framework with `P(m|s)` as slow-loop objective.
2. **Five-agent pipeline**: notes that Retriever `σ` and Allocator `α` are the *two typed stages* they add on top of the standard analyze→propose→evolve loop. This is the first honest admission that the pipeline is not just a rename of prior work — two stages are new.
3. Recursive self-improvement via typed meta-skills; explicitly "bounded, one-level recursion."
4. Meta-aware frontier selection combining `U`, `P̂`, `N`.

**What I notice:**
- Contribution 2 quietly weakens the "same pipeline" claim from earlier. Prior systems had analyze→propose→evolve; MetaSkill-Evolve adds Retriever and Allocator. So "the same five-agent pipeline that rewrites `s`" is trivially true only because *they defined it that way* — they built the pipeline they then reapply.
- "Bounded, one-level recursion" is a defensible claim but also a limitation: they do not evolve the meta-meta-skill. Nothing in principle forbids it; they just stop.

---

## What sticks after §1

- The critique of prior work is that the *operator* is padlocked. The remedy is to make the operator a *file*, then reapply the operator to itself.
- Two quantities are separated: `U(s)` (what the skill scores) and `P(m|s)` (how fast the branch generates better children). This distinction is the paper's conceptual pivot.
- The five-agent pipeline (`ψ, σ, α, π, ε`) is *engineered* to match the paper's critique of prior fixed-meta systems — the five things prior systems freeze are exactly the five things this system makes learnable.
- The recursion is deliberately bounded to one level. The paper does not claim to solve unbounded recursive self-improvement; it claims a *practical, bounded* instance.
- Two rhetorical moves are worth flagging as I move to §3:
  1. "Same pipeline" is true by construction — they built the pipeline they reapply.
  2. "No new objective" glosses the introduction of `P(m|s)`.
