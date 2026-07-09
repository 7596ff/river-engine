# Abstract + Figure 1 (paragraph-level argumentative)

## ¶ Abstract — "Recent LLM agents tackle…"

**What this paragraph does:**
- Sets up the domain: LLM agents + external "skills" as reusable procedural knowledge.
- Names the gap: existing self-improving systems rewrite the *skill* but not the *rewriter*. They call this "non-recursive."
- Announces the fix: a two-timescale framework where every branch carries `s` (task skill) *and* `m = (ψ, σ, α, π, ε)` (meta-skill), and the *same* pipeline is applied to itself to refine `m`.
- Names the numbers: +23.54 / +16.09 / +1.92 on OfficeQA / SealQA / ALFWorld over the raw backbone.

**Key distinctions:**
- **skill**: "reusable procedural knowledge supplied to the agent" — the paper insists it is a *file-system artifact*, a Markdown spec, not just a prompt.
- **task skill vs. meta-skill**: "what the agent does" vs. "how it improves." This *what/how* framing is the paper's central rhetorical spine — watch for it recurring.
- **self-improving vs. recursively self-improving**: the paper draws this line very deliberately, invoking Good (1965) and Schmidhuber later. Non-recursive = the operator that optimizes stays fixed.
- **fast / slow timescale**: `s` evolves every iteration; `m` evolves every `H` iterations.

**What I notice:**
- The claim "with no additional model or objective" is doing a lot of work but is only half-true: they *do* introduce a new objective — meta-productivity `P(m|s)` — later in §3.2. It is folded into a single frontier score with utility, but calling it "no additional objective" glosses that it is a distinct quantity with its own definition and estimator.
- "The same pipeline applied to itself" is the claim to underline; it is what makes the recursion feel cheap. The representational trick (meta-skills are Markdown, same as task skills) is what makes it work at the implementation level.
- The five-tuple `(ψ, σ, α, π, ε)` shows up whole, not built up. The Greek letters do not yet mean anything to the reader — they will be defined in §3.2.

---

## Figure 1 — Four regimes

The caption sequences four boxes:
1. **No-Skill** — no reusable memory.
2. **Static Skill** — hand-authored `s₀`, "padlocked."
3. **Single-Level Evolve** — `s₀ → s₁ → s₂`, "but the driving meta-process stays padlocked."
4. **MetaSkill-Evolve** — meta-skill co-evolves on a slower outer ring "via the same five-agent pipeline that rewrites `s`, with no extra model and no extra framework."

**What I notice:**
- The rhetorical device is the **padlock**. Each intermediate regime removes one padlock. The paper is arguing progress by unlocking successively more of the system.
- "Outer ring" imagery: fast loop = inner ring, slow loop = outer ring. Concentric, not nested. This matters — the paper will not recurse a third level; the recursion is *bounded to one level*.
- "No extra model, no extra framework" is the paper's cost claim. Combined with "same pipeline," it is arguing this is a *free* upgrade.

---

## Running vocabulary (established so far)

| Term | Definition |
|---|---|
| skill | Editable Markdown spec of reusable procedures; a file-system artifact. |
| task skill `s` | The skill the agent executes to do the task. |
| meta-skill `m` | Five-tuple `(ψ, σ, α, π, ε)`; parameterises the improvement pipeline. |
| branch | A lineage; each branch carries its own `(s, m, h)`. |
| fast / slow timescale | Every iteration / every `H` iterations. |
| recursively self-improving | Improves the improvement operator, not just the artifact. |

## What sticks

- The whole paper hangs on one *representational* claim: because the meta-skill is a Markdown file just like a task skill, the operator can be applied to itself with no new machinery. Everything else — the timescales, the frontier score, the ablations — is downstream of that.
- The "padlock" framing is doing quiet argumentative work: it makes non-recursive systems look *artificially* constrained, as if there is no principled reason to stop at level 1. But the paper itself stops at level 1 (bounded one-level recursion) — so the padlock is only removed one notch, not eliminated.
