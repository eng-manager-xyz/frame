---
name: writing
description: Write, edit, or review human-facing prose for clarity, accuracy, audience fit, and purpose. Use when the requested deliverable primarily involves drafting, editing, or reviewing documentation, READMEs, comments or docstrings, PR or issue descriptions, commit messages, release notes, status updates, plans, reports, UI or error text, Slack or email drafts, marketing or landing-page copy, blog posts, or other prose; do not wait for a request for “good writing.” Do not trigger solely for code, identifiers, commands, schemas, exact quotations, fixed legal text, source material supplied only as implementation context, or routine conversation where prose is incidental. Preserve explicit tone, house style, facts, format, and length.
---

# Writing

Produce prose that its intended reader can understand and use on the first
reading. Treat the 21 rules as revision heuristics, not permission to change
meaning or flatten the writer's voice.

## Set the writing contract

Before drafting or editing, resolve the smallest useful contract:

- Identify the audience, artifact, purpose, desired reader action, channel,
  tone, length, format, locale, and applicable house style.
- Identify facts, evidence, uncertainty, citations, links, quotations, and text
  that must remain exact.
- Infer missing details from the request and repository when the answer is
  clear. Ask only when a missing choice would materially change the result.
- Preserve the source's factual claims, numbers, names, logical polarity, scope,
  modality, commitments, degree of certainty, technical meaning, genre, and
  voice unless the user asks to change them.
- In factual artifacts, never invent numbers, dates, owners, deadlines, test
  results, quotations, customer outcomes, product capabilities, or proof. When
  the user requests fiction, hypotheticals, examples, or placeholders, keep
  invented details clearly fictional or labeled and never present them as proof.
- Do not promote a warning, fallback, example, observation, correlation, or
  optional action into a prerequisite, guarantee, cause, or command.

Apply this priority order when guidance conflicts:

1. Truth, safety, permissions, and required content
2. Explicit user instructions and supplied facts
3. Repository, brand, legal, accessibility, localization, and artifact standards
4. The intended audience, purpose, genre, and voice
5. The rules in this skill

Do not alter code, identifiers, commands, paths, URLs, API names, configuration
keys, issue IDs, data formats, exact errors, quotations, citations, placeholders,
or text explicitly marked fixed. Preserve code spans and fences byte-for-byte
unless the task targets them. When UI or error text is the target, edit the
human-facing string while preserving surrounding syntax and interpolation tokens.

For non-English prose, follow the target language's norms. Apply only this
skill's language-independent fidelity, audience, evidence, and structure rules
unless the user asks for an English adaptation.

## Apply the 21 rules

### Diction

1. Cut dead metaphors, stock similes, and familiar figures that make the reader
   stop seeing the subject. Keep or create a figure only when it clarifies the
   idea or carries intentional voice.
2. Prefer the shortest familiar word that preserves the exact meaning. Precision
   beats shortness.
3. Cut every word that adds no meaning, logic, tone, or necessary rhythm.
4. Prefer active voice when the actor or responsibility matters. Use passive
   voice when the actor is unknown, irrelevant, deliberately de-emphasized, or
   less important than the object or result.
5. Replace needless jargon with language the audience knows. Preserve required
   scientific, legal, domain, and product terms; define them on first use when
   the audience needs the definition.

### Structure

6. Lead with the point in operational prose and at the start of each explanatory
   paragraph. Let narrative and persuasive openings create context or tension
   when that structure serves the reader.
7. Put the actor in the subject and the action in the verb: “The race interrupted
   deployment,” not “A deployment interruption occurred.”
8. Prefer a strong verb to a nominalization: “decide,” not “make a decision.”
9. Prefer the positive, direct form: “rejects,” not “does not accept.” Preserve
   negative wording when a prohibition, boundary, or warning is the point.
10. Cut empty hedges and intensifiers such as “very,” “really,” “quite,” and
    “arguably.” Preserve material uncertainty and name its source: “untested on
    Safari,” not “should mostly work.”
11. Prefer verified numbers, conditions, and examples to vague adjectives. Never
    invent a measurement; keep a precise qualitative statement when no useful
    measure exists.
12. Use one name for one thing. Change terms only when the distinction is real or
    repetition would damage intentional voice.
13. Keep one main idea per sentence. Join tightly coupled ideas when separation
    would make the relationship harder to understand.
14. Delete wind-ups such as “This PR aims to,” “It should be noted that,” and “In
    order to.” Start where the information starts.
15. Make the status or requested action explicit. Include an owner and deadline
    only when supplied or verified. Put the ask where the reader will see it;
    close a longer action-oriented message with the ask or next step. Do not
    manufacture an ask for informational prose; label it “FYI” only when useful.
16. Run a cadence pass. Read the text as spoken prose and rewrite any unintended
    stumble, ambiguity, or monotonous run. Do not claim audible playback unless
    it occurred.

### Claims

Apply rules 17–20 to persuasive claims about value, quality, performance, or
difference. Technical facts still need evidence, but they do not need to be
visual or unique to one company. In a style-only copyedit, preserve substantive
claims and flag unsupported or excessive wording separately. Narrow, qualify,
or remove a supplied claim only when the task authorizes substantive claim
editing; report the material change when the user needs to approve it.

17. Make the claim concrete enough to picture or observe. Prefer a scene,
    behavior, outcome, or example to an abstract compliment.
18. Make the claim falsifiable. Verify each objective claim you create or
    substantively revise against evidence supplied or retrieved. Cite or link
    the proof when the artifact permits. For a style-only edit, follow the scope
    rule above instead of silently changing an unverified supplied claim.
19. Make a differentiating claim ownable. If any competitor could use the same
    sentence unchanged, name the specific mechanism, audience, proof, or tradeoff.
20. Test each persuasive claim: **Can the reader picture or observe it? Can
    evidence verify it? Is it specific to this product or position?** Use the
    answers as diagnostics, not a mechanical grade. A claim that fails all three
    is presumptively weak. When substantive claim editing is in scope, narrow,
    substantiate, rewrite, or cut it unless accuracy or genre requires the
    literal statement. Otherwise flag it separately. Never turn a goal,
    estimate, or intention into a proven result.

### Escape hatch

21. Break any rule when following it would reduce accuracy, safety, clarity,
    accessibility, localization, required tone, or fitness for the artifact.
    State the tradeoff only when the user needs to review it.

## Respect the artifact

- **Documentation and READMEs:** Organize around the reader's task. When relevant
  and supported, include the outcome, prerequisites, steps, expected result, and
  recovery path. Keep code and technical terms exact.
- **PRs, issues, commits, and release notes:** State what changed and why. Include
  scope, validation, risk, migration, or breaking impact when relevant. Never
  claim a check ran when it did not.
- **Status updates and reports:** Put results, decisions, risks, and blockers
  first. Tie evidence to the claim. Name owners and dates only from the source.
- **Slack and email:** Put the decision or request early, add the minimum context,
  and make the next step visible. Keep courtesy that fits the relationship.
- **UI and error text:** Name what happened, what the reader can do, and any
  irreversible consequence. Avoid blame and false reassurance.
- **Marketing, landing pages, and announcements:** Use the audience's language,
  verified proof, specific value, and a clear action. Keep personality. Reject
  fabricated urgency, metrics, testimonials, and exclusivity. Require a
  reasonable evidentiary basis before making an express or implied commercial
  claim, and route claims through applicable legal or brand review.
- **Blogs and narrative prose:** Preserve the writer's point of view, pacing, and
  texture. Remove clutter without sanding every sentence into the same rhythm.

Use headings, lists, or tables only when they make scanning easier. Keep short
messages as prose when structure would add more weight than clarity.

## Review before delivery

Run one bounded revision loop:

1. **Fidelity:** Compare the draft with the source. Restore any changed fact,
   uncertainty, technical token, quotation, constraint, or intended voice.
2. **Shape:** Check that the opening, order, headings, paragraphs, and ending
   serve the audience's task.
3. **Sentences:** Apply rules 1–16 without making the prose abrupt or robotic.
4. **Claims:** Extract persuasive claims and apply rules 17–20. When substantive
   claim editing is in scope, narrow or label unsupported claims instead of
   making them sound stronger. Otherwise preserve and flag them separately.
5. **Finish:** Check grammar, parallel structure, links, accessibility,
   localization risks, channel length, and cadence.

Stop when the prose is accurate, audience-ready, easy to scan at the required
depth, and explicit about any action or uncertainty. Further changes that only
swap one acceptable preference for another are not an improvement.

Deliver the requested artifact without an editing lecture. Explain material
choices, preserved ambiguities, or missing evidence only when the user asked for
commentary or needs that information to use the text safely.

## Examples

**PR description**

Before: “This PR aims to implement a fix for an issue where deployments were
occasionally observed to fail due to a race condition.”

After: “Proposes a fix for a race that intermittently failed deployments.”

If review confirms the fix and logs establish a rate, write: “Fixes the
deployment race that failed 3 of 100 staging releases.”

**Status update**

Before: “We made significant progress on performance improvements this sprint.”

After, when the measurement is supplied: “Checkout p95 fell from 1.8 s to 1.1 s
this sprint.”

**Message with an ask**

Before: “Just wanted to flag that the ERD sourcing question might need some
discussion at some point.”

After: “ERD sourcing may need discussion.” If context establishes that a
decision is required, say so. Add options, an owner, and a date only if the
source provides them.

**Marketing claim**

Before: “The most powerful CMS for modern teams.”

After, when a verified case study supports every detail: “With [Product]'s
batch publisher, [Customer]'s two-person team publishes 400 landing pages a
day.” Replace the placeholders from the source and link the case study when the
artifact permits.

## Research provenance

Read [sources.md](references/sources.md) only when auditing or updating this
skill's rules. Do not load it for an ordinary writing task.
