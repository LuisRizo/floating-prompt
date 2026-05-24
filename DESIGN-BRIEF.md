# Floating-Prompt — UI Design Brief

A Windows-native floating popup that mediates between Claude Code (running
in a terminal/IDE) and the human. When Claude finishes a turn or needs a
decision, this popup appears on top of whatever the user is doing, waits
for an answer, then hands it back to Claude so the turn continues. It is
the **only** synchronous interaction surface for a Claude Code session.

This brief is for redesigning the popup's visual + interaction layer. The
underlying behavior (queueing, double-Esc dismissal, returning the answer
to Claude) is fixed by an existing contract — don't redesign those.

---

## 1. Usage flow

1. Claude Code runs a turn. At a decision point (turn complete, or Claude
   asks a question, or Claude proposes a plan), a hook fires and spawns
   the popup process.
2. The popup appears **on top, without stealing keyboard focus** (the user
   may be typing elsewhere). A single click activates it; until then, it
   sits politely above other windows.
3. The user reads the message and either:
   - Picks one of the offered options (if any), OR
   - Types a free-text reply and presses Enter, OR
   - Presses **Esc twice within ~600 ms** (works globally — does not
     require the popup to have focus) to dismiss without answering.
4. The popup closes and the answer (or "dismissed") is sent back to
   Claude, which continues the turn.

Multiple Claude Code sessions can hit hooks concurrently. Only one popup
is visible at any moment; later ones queue behind the active one and
appear in order as each prior popup is resolved.

---

## 2. Sources of the popup (what Claude is asking for)

The popup must handle every shape of "Claude needs me." Concretely:

| Trigger | Has options? | Typical message body | Notes |
|---|---|---|---|
| **Stop** (Claude finished a turn) | No | Claude's last assistant message — can be one line OR many paragraphs (full plan / diff explanation / status report) | User can reply to keep the turn going, or dismiss to let it end. The message can be very long. |
| **AskUserQuestion** — single select | 2–4 labeled options | The question text. Often one sentence; sometimes a short paragraph with context | User picks one option OR types free text (counts as "Other"). |
| **AskUserQuestion** — multi select | 2–4 labeled options, multi | Same as above | User picks any subset. Visually distinct from single-select (checkboxes vs. radio-style buttons). |
| **AskUserQuestion** — with previews | 2–4 labeled options, each with a preview block (code snippet, mockup, diagram in ascii/text) | The question text | When the focused option has a preview, it should be visible alongside the option list. |
| **ExitPlanMode** | Exactly one option: "Approve" | The plan text — usually multi-paragraph | User clicks Approve to ship, OR types changes as free text. |

Option labels are author-supplied strings of variable length, anywhere
from one word ("Allow", "Approve") to a full sentence ("Roll back the
whole migration and retry from scratch"). The design must handle both
gracefully.

---

## 3. UI elements (the only things on screen)

The popup is intentionally minimal. The redesign should contain **exactly
these six elements** — nothing else. No settings cog, no theme picker, no
branding, no help link. Configuration happens through a separate Claude
Code skill, not through controls on the popup itself.

### 3.1 Session context (top-left)

A small, low-noise indicator that tells the user **which Claude session
this popup belongs to**. When the user has 3–4 Claude Code sessions
running in different repos, this is how they know what they're answering.

Available data points (the designer picks how to use them):
- **Project name** — basename of the working directory (e.g. `claude-integration`)
- **Palette / accent color** — assigned per-project via the skill (see §5).
  Can be used as a chip, border accent, or icon background.
- **Session ID** — opaque short hash; only useful if multiple sessions
  share the same project (rare but possible).

Tone: glanceable, not loud. Should not compete with the message body.
Typical width: 30–40% of the popup's title row.

### 3.2 Queue indicator (top-right)

When there are N popups waiting (this one plus N-1 others queued):
- N == 1: **hidden entirely** (no element, no placeholder).
- N >= 2: a small badge showing just the number `N`. No "X of Y" label,
  no "queue:" prefix. Just `2` or `3`. Updates live as siblings arrive
  or get answered.

### 3.3 Message body (the response or request from Claude)

The main content area. Variable size: from a single line ("Done.") to
many paragraphs (a full migration writeup).

Requirements:
- **Vertically resizes the popup up to a max height** (~720 px today).
  Below the max, the body grows to fit so no scroll is needed.
- **Scrollable when content exceeds max height.** Mouse wheel, drag the
  scrollbar, keyboard arrow keys when focused.
- **Text is selectable** so the user can copy snippets out (code,
  filenames, command examples).
- **Paragraph breaks render** (Claude's responses often use blank lines
  between paragraphs).
- **Future-deferred:** markdown / fenced code blocks render with
  appropriate styling. Not required in this pass but the layout should
  not preclude it (treat the message area as a content region the
  designer may later replace with a rich-text view).
- Font: comfortable for reading multi-paragraph prose. Not the same as
  the system UI font necessarily.

### 3.4 Options (zero to four)

Below the message body, when Claude offered options. **Vertically
stacked, full content-width, dynamically sized to fit their labels.**

States to design:
- **No options** → element absent entirely; message body expands.
- **1 option** (ExitPlanMode "Approve") → single button.
- **2–4 single-select options** → stacked clickable rows. Numbered
  `1.` … `4.` on the left so the user can later type the digit to pick
  (future: deferred).
- **2–4 multi-select options** → stacked rows with a checkbox affordance;
  the bottom of the list has an implicit "submit selection" via Enter.
  When zero are checked and the user presses Enter, treat as free-text
  submission instead.
- **Options with previews** → the preview content (code/mockup/diagram)
  appears in a panel next to or below the focused option. Hovering or
  arrow-key-focusing an option swaps the preview.

Labels can be one word or a full sentence. Long labels should wrap inside
the button rather than truncate.

Visual weight: distinguishable from chrome but quieter than the message
text — the user's attention should land on the message first, then the
options.

### 3.5 Free-text input

A single-line (auto-growing? to consider) edit field below the options.

- Always present, even when options exist (every state allows free-text).
- Placeholder text suggests context-appropriate prompt — e.g. for Stop:
  "Reply to continue, or double-Esc to let Claude stop"; for
  AskUserQuestion: "Type a custom answer…"; for ExitPlanMode: "Or describe
  changes to the plan…".
- On Enter: submit the typed text. If empty, do nothing (don't submit empty).
- Tab order: starts here when the popup is first interacted with.

### 3.6 Dismiss control + keyboard legend (below input — replaces the old Send button)

A single quiet row that serves two purposes: it reminds the user of the
keys (Enter to send, Esc-Esc to dismiss) AND offers a clickable Dismiss
affordance for users who'd rather mouse. There is **no Send button** —
Enter is the only submit path. The Dismiss control is the only clickable
element in this row.

Visual weight: footer-grade. Smaller than body text, dim. On a fresh
popup it should not draw the eye — it's a reminder, not a CTA.

**Layout idea (designer's call on exact form):**

```
Enter to send          [ Esc · Esc  Dismiss ]
```

The right-hand cluster is the Dismiss button. It visually represents
the Esc-Esc gesture so the legend and the button are the same element:
two Esc "pips" or chips followed by the word Dismiss. Clicking anywhere
on the cluster dismisses immediately (no double-click needed — mouse
users get a single click; the double-tap only applies to the keyboard
shortcut). Keyboard-only users press Esc twice anywhere on the screen.

**Animated state for the Esc-Esc gesture (this is the key UX detail):**

The Dismiss control mirrors the state of the global double-Esc detector
so the user sees their first Esc press was registered, and sees it
expire if they don't follow through.

1. **Resting state** — both Esc pips dim, label "Dismiss" dim. Looks
   like static legend text.
2. **First Esc pressed (within the window)** — pip 1 lights up / fills
   with the accent color; pip 2 stays dim. A subtle progress indicator
   (thin bar under the pips, or pip 1's fill draining over ~600 ms) shows
   the time remaining for the second Esc.
3. **Second Esc pressed in time** — both pips light briefly, then the
   popup dismisses. The visual "completion" should be perceptible (~80 ms)
   so the user sees their action succeeded.
4. **Timeout** — if the second Esc doesn't arrive within ~600 ms, the
   progress indicator visibly empties and pip 1 returns to resting state.
   This is the critical feedback: it tells the user "your first Esc has
   expired, you'd have to start over."

Tone for the animation: small, calm. No bounce, no flash. A linear fill
that drains is fine. The user should be able to ignore it entirely on a
normal interaction — it's only attention-grabbing when they've started
the gesture and walked away.

The control's state is driven by the same global keyboard hook that
implements the double-Esc dismiss (existing in the .exe). The popup
doesn't need keyboard focus for the visual to update — pressing Esc with
focus on a different window still lights up pip 1.

---

## 4. Behavioral contract (don't change these — design around them)

- **Stays on top** of all other windows until dismissed/answered.
- **Does not steal focus** when it appears. The user's current keyboard
  focus (terminal, browser, IDE) is preserved. They click the popup
  themselves when they want to type into it.
- **Borderless / no title bar** (the popup IS the title bar — drag
  region is in the upper chrome area, not the message body).
- **Draggable** by clicking-and-holding any non-content area of the
  upper portion. Last position persists across invocations and restarts,
  per-machine. The popup remembers where the user last put it.
- **Default position** for first-time users: bottom-right of the primary
  monitor's work area, with a margin.
- **Enter submits** the free-text input. **Double-Esc within ~600 ms**
  dismisses (works even when the popup doesn't have focus — there is a
  global low-level keyboard hook).
- **Width range:** ~480–640 px. Grows with the longest option label
  to avoid cramped buttons, never wider than 640.
- **Height range:** ~180 px (floor) to ~720 px (cap). Above the cap, the
  message body scrolls inside the popup.

---

## 5. Theming — per-project color palettes (configured externally)

Each project (working directory) can have its own color palette. This
lets the user visually distinguish at-a-glance which session a popup
belongs to: `claude-integration` might be teal, `backend-api` might be
amber, `personal-notes` might be slate.

**The popup must not contain any UI to pick or change palettes.** No
swatches, no settings cog, no dropdown. Palettes are assigned through a
Claude Code **skill** (a slash command), e.g.:

```
/floating-prompt palette ocean
/floating-prompt palette amber
/floating-prompt palette default
```

The skill writes the assignment into the popup's config file
(`%LOCALAPPDATA%\floating-prompt\state.json` or sibling). The popup reads
the assignment on launch and applies the palette.

**Design deliverable for theming:**

1. A **palette schema** — the set of color slots a palette defines.
   Examples to consider:
   - Background (main popup surface)
   - Message panel (subtle tint behind the message body)
   - Accent (chip/border for session context, focus rings)
   - Option button surface (rest, hover, pressed)
   - Option button label text
   - Body text
   - Title text
   - Dim text (queue counter, keyboard legend)
   - Scrollbar thumb
2. A **set of 4–6 ready-made palettes** that ship by default. They should
   feel cohesive as a family but distinguishable at-a-glance. Suggested
   names: slate, ocean, amber, forest, plum, default-dark. Each palette
   provides values for every slot in the schema.
3. **Contrast / readability constraints** — all palettes must meet WCAG
   AA contrast for body text vs. background and option label vs. button
   surface. Designed for dark-mode-first; light-mode is out of scope for
   this pass.

The palette mechanism shapes the design's structure: the designer should
ensure no color is hard-coded into mockups except as palette slot
references.

---

## 6. Anti-requirements (explicitly NOT in scope)

- No on-popup theme/palette picker, settings panel, or preferences UI.
- No "About" / branding / version indicator visible to the user.
- No persistent system tray icon or background presence (the .exe runs
  only while a popup is live).
- No "submit" button — Enter is the only submit path.
- No "X of Y" queue label — just the bare count when > 1.
- No markdown / rich text rendering in this pass (designed-around, not
  implemented).
- No voice input affordance on the popup (deferred).
- No mobile / web variant — Windows-native only.

---

## 7. Deliverables wanted from the design pass

1. **High-fidelity mockups** of these states (one frame each unless noted):
   - Stop, short reply, no queue.
   - Stop, multi-paragraph reply (scrollable body), no queue.
   - AskUserQuestion, 3 short options, no queue.
   - AskUserQuestion, 3 sentence-length options, queue count = 3.
   - AskUserQuestion, multi-select, 4 options.
   - AskUserQuestion with previews, focused option showing its preview.
   - ExitPlanMode, plan body + single Approve option.
2. **Palette family** — 4–6 dark palettes with named slots, applied to
   one of the mockups above to demonstrate variety.
3. **Hover / focus states** for option buttons.
4. **Empty-input vs filled-input** state for the free-text field.
5. **Drag affordance** — how the user discovers the popup is draggable
   without a title bar.
6. **Dismiss control — all four states** in a small filmstrip:
   resting → first-Esc-armed (with the draining progress indicator) →
   second-Esc-completing → timeout-returning-to-rest. The frames should
   make the ~600 ms window legible at a glance.

---

## 8. Tone notes for the designer

- The popup is a **conversation moment, not a tool window.** It should
  feel like a quiet message arriving, not like a system dialog
  demanding action.
- Density: the user may answer 30+ of these per day. Visual fatigue
  matters. Avoid loud chrome, heavy borders, decorative gradients.
- The user's primary focus is the **message body**. Options and chrome
  support that, they don't compete with it.
- This is a power-user tool. Assume keyboard fluency. Make the keyboard
  affordances visible (the legend) but unobtrusive.
