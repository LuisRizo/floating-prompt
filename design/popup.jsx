// Floating-Prompt — Popup component
// All visual states of the popup, parameterized by `state` prop.
// Used inside DesignCanvas artboards. Pure render; the only "live"
// interactions are hover/focus visuals so the user can see those states
// when mousing the mockups.

const FP_W = 520; // popup width (within brief's 480-640 range)

// ---------- styles (one shared sheet, theme via inline CSS vars) ----------
if (!document.getElementById('fp-styles')) {
  const s = document.createElement('style');
  s.id = 'fp-styles';
  s.textContent = `
@import url('https://fonts.googleapis.com/css2?family=Geist:wght@400;500;600&family=Geist+Mono:wght@400;500&display=swap');

.fp-root {
  width: ${FP_W}px;
  background: var(--fp-bg);
  border-radius: 14px;
  box-shadow:
    0 1px 0 0 rgba(255,255,255,0.04) inset,
    0 0 0 1px rgba(255,255,255,0.06),
    0 24px 60px -12px rgba(0,0,0,0.55),
    0 8px 24px -8px rgba(0,0,0,0.4);
  font-family: 'Geist', -apple-system, 'Segoe UI', system-ui, sans-serif;
  font-feature-settings: 'cv11','ss01';
  color: var(--fp-body);
  padding: 14px 14px 12px 14px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  position: relative;
  overflow: hidden;
}

/* Top row: session chip · drag grip (center) · queue badge */
.fp-top {
  display: grid;
  grid-template-columns: 1fr auto 1fr;
  align-items: center;
  height: 24px;
  cursor: grab;
  user-select: none;
}
.fp-top:active { cursor: grabbing; }

/* Session chip — colored dot + project name + optional session id */
.fp-chip {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 4px 10px 4px 8px;
  background: var(--fp-chip);
  border: 1px solid var(--fp-chip-border);
  border-radius: 999px;
  height: 24px;
  max-width: fit-content;
  justify-self: start;
}
.fp-chip-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--fp-accent);
  flex: 0 0 auto;
  box-shadow: 0 0 0 3px var(--fp-accent-soft);
}
.fp-chip-name {
  font-size: 12px;
  font-weight: 500;
  color: var(--fp-title);
  letter-spacing: -0.005em;
  line-height: 1;
}
.fp-chip-sep {
  width: 1px;
  height: 10px;
  background: var(--fp-chip-border);
}
.fp-chip-hash {
  font-family: 'Geist Mono', ui-monospace, monospace;
  font-size: 10.5px;
  color: var(--fp-dim);
  line-height: 1;
  letter-spacing: 0;
}

/* Drag grip — quiet 4-dot affordance, brightens on hover of the row */
.fp-grip {
  display: grid;
  grid-template-columns: repeat(4, 3px);
  gap: 4px;
  opacity: 0.32;
  transition: opacity .15s;
}
.fp-top:hover .fp-grip { opacity: 0.72; }
.fp-grip-dot {
  width: 3px;
  height: 3px;
  border-radius: 50%;
  background: var(--fp-dim);
}

/* Queue badge — only rendered when count >= 2 */
.fp-queue {
  justify-self: end;
  min-width: 22px;
  height: 22px;
  padding: 0 7px;
  border-radius: 6px;
  background: var(--fp-chip);
  border: 1px solid var(--fp-chip-border);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-family: 'Geist Mono', ui-monospace, monospace;
  font-size: 12px;
  font-weight: 500;
  color: var(--fp-title);
  letter-spacing: 0;
}

/* Message body panel */
.fp-panel {
  background: var(--fp-panel);
  border-radius: 10px;
  padding: 14px 16px 14px 16px;
  font-size: 14.5px;
  line-height: 1.55;
  color: var(--fp-body);
  max-height: 360px;
  overflow-y: auto;
  scrollbar-width: thin;
  scrollbar-color: var(--fp-scroll) transparent;
  letter-spacing: -0.003em;
  -webkit-user-select: text;
  user-select: text;
}
.fp-panel.fp-panel-tall { max-height: 520px; }
.fp-panel.fp-panel-short { max-height: none; }
.fp-panel p { margin: 0; }
.fp-panel p + p { margin-top: 0.9em; }
.fp-panel code, .fp-panel .fp-code {
  font-family: 'Geist Mono', ui-monospace, monospace;
  font-size: 0.88em;
  background: rgba(255,255,255,0.05);
  padding: 1px 5px;
  border-radius: 4px;
  letter-spacing: 0;
}
.fp-panel::-webkit-scrollbar { width: 8px; }
.fp-panel::-webkit-scrollbar-thumb {
  background: var(--fp-scroll);
  border-radius: 4px;
  border: 2px solid var(--fp-panel);
  background-clip: padding-box;
}
.fp-panel::-webkit-scrollbar-track { background: transparent; }

/* Options list */
.fp-opts {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.fp-opt {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  background: var(--fp-opt-bg);
  border: 1px solid var(--fp-opt-border);
  border-radius: 8px;
  padding: 10px 14px;
  font-size: 13.5px;
  line-height: 1.45;
  color: var(--fp-body);
  cursor: pointer;
  text-align: left;
  transition: background .12s, border-color .12s, transform .08s;
  font-family: inherit;
  letter-spacing: -0.003em;
}
.fp-opt:hover, .fp-opt.is-hover {
  background: var(--fp-opt-hover);
  border-color: var(--fp-opt-border);
}
.fp-opt.is-focus, .fp-opt:focus-visible {
  outline: none;
  border-color: var(--fp-accent);
  box-shadow: 0 0 0 3px var(--fp-accent-soft);
}
.fp-opt-num {
  font-family: 'Geist Mono', ui-monospace, monospace;
  font-size: 11.5px;
  color: var(--fp-opt-num);
  flex: 0 0 auto;
  padding-top: 2px;
  min-width: 14px;
}
.fp-opt-label {
  flex: 1 1 auto;
  min-width: 0;
}
.fp-opt-check {
  flex: 0 0 auto;
  width: 16px;
  height: 16px;
  border-radius: 4px;
  border: 1.5px solid var(--fp-opt-border);
  margin-top: 1px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
}
.fp-opt.is-checked .fp-opt-check {
  background: var(--fp-accent);
  border-color: var(--fp-accent);
}
.fp-opt-check svg { display: block; }

/* Approve — special single-option full-width primary */
.fp-opt-approve {
  justify-content: center;
  background: var(--fp-accent);
  border-color: var(--fp-accent);
  color: #0d0e10;
  font-weight: 500;
  padding: 11px 14px;
}
.fp-opt-approve:hover, .fp-opt-approve.is-hover {
  background: var(--fp-accent);
  filter: brightness(1.08);
}
.fp-opt-approve .fp-opt-num { display: none; }

/* Options with previews: two-column layout */
.fp-opts-preview {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1.05fr);
  gap: 10px;
  align-items: stretch;
}
.fp-preview {
  background: var(--fp-panel);
  border-radius: 8px;
  padding: 12px 14px;
  font-family: 'Geist Mono', ui-monospace, monospace;
  font-size: 11px;
  line-height: 1.55;
  color: var(--fp-body);
  white-space: pre;
  overflow: auto;
  scrollbar-width: thin;
  scrollbar-color: var(--fp-scroll) transparent;
}
.fp-preview-label {
  font-family: 'Geist', sans-serif;
  font-size: 10.5px;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: var(--fp-dim);
  margin-bottom: 8px;
}

/* Free-text input */
.fp-input-wrap {
  position: relative;
  display: flex;
  align-items: center;
}
.fp-input {
  width: 100%;
  background: var(--fp-input-bg);
  border: 1px solid var(--fp-input-border);
  border-radius: 8px;
  color: var(--fp-body);
  font-family: inherit;
  font-size: 13.5px;
  padding: 10px 13px 10px 13px;
  outline: none;
  letter-spacing: -0.003em;
  transition: border-color .12s, box-shadow .12s;
}
.fp-input::placeholder { color: var(--fp-dim); }
.fp-input:focus, .fp-input.is-focus {
  border-color: var(--fp-accent);
  box-shadow: 0 0 0 3px var(--fp-accent-soft);
}
.fp-input-caret {
  position: absolute;
  pointer-events: none;
  left: 13px;
  width: 1px;
  height: 16px;
  background: var(--fp-body);
  opacity: 0.8;
  animation: fp-blink 1s steps(2) infinite;
}
@keyframes fp-blink { 50% { opacity: 0; } }

/* Footer row */
.fp-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 2px;
  font-size: 11.5px;
  color: var(--fp-dim);
  height: 24px;
  letter-spacing: 0.005em;
}
.fp-foot kbd, .fp-pip {
  font-family: 'Geist Mono', ui-monospace, monospace;
  font-size: 10px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 32px;
  height: 18px;
  padding: 0 6px;
  border-radius: 4px;
  background: var(--fp-chip);
  color: var(--fp-dim);
  border: 1px solid var(--fp-chip-border);
  border-bottom-width: 2px;
  line-height: 1;
}

/* Dismiss cluster — keyboard legend + clickable target */
.fp-dismiss {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 3px 8px 3px 6px;
  border-radius: 8px;
  cursor: pointer;
  background: transparent;
  border: 0;
  color: inherit;
  font: inherit;
  transition: background .12s;
}
.fp-dismiss:hover { background: var(--fp-chip); }
.fp-dismiss-pips {
  display: inline-flex;
  align-items: center;
  gap: 3px;
}
.fp-pip {
  min-width: 22px;
  height: 18px;
  padding: 0 5px;
  font-size: 9.5px;
  transition: background .12s, color .12s, border-color .12s, transform .12s;
}
.fp-pip.is-armed {
  background: var(--fp-accent);
  color: #0d0e10;
  border-color: var(--fp-accent);
}
.fp-pip.is-done {
  background: var(--fp-accent);
  color: #0d0e10;
  border-color: var(--fp-accent);
  transform: scale(1.06);
}
.fp-dismiss-progress {
  display: block;
  width: 100%;
  height: 2px;
  margin-top: 2px;
  background: var(--fp-chip-border);
  border-radius: 1px;
  overflow: hidden;
}
.fp-dismiss-progress > i {
  display: block;
  height: 100%;
  background: var(--fp-accent);
  transform-origin: left center;
}
.fp-dismiss-label {
  color: var(--fp-dim);
  font-size: 11.5px;
  font-weight: 500;
  letter-spacing: 0.005em;
}
.fp-dismiss-pipswrap {
  display: inline-flex;
  flex-direction: column;
  align-items: stretch;
  min-width: 56px;
}
`;
  document.head.appendChild(s);
}

// ---------- helpers ----------
function paletteVars(palette) {
  const p = palette;
  return {
    '--fp-bg':           p.bg,
    '--fp-panel':        p.panel,
    '--fp-chip':         p.chip,
    '--fp-chip-border':  p.chipBorder,
    '--fp-accent':       p.accent,
    '--fp-accent-soft':  p.accentSoft,
    '--fp-opt-bg':       p.optionBg,
    '--fp-opt-hover':    p.optionHover,
    '--fp-opt-border':   p.optionBorder,
    '--fp-opt-num':      p.optionNumber,
    '--fp-input-bg':     p.inputBg,
    '--fp-input-border': p.inputBorder,
    '--fp-body':         p.body,
    '--fp-title':        p.title,
    '--fp-dim':          p.dim,
    '--fp-scroll':       p.scrollThumb,
  };
}

function Check() {
  return (
    <svg viewBox="0 0 12 12" width="11" height="11" fill="none">
      <path d="M2.5 6.2 L5 8.5 L9.5 3.5" stroke="#0d0e10" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

// ---------- subcomponents ----------
function SessionChip({ project, sessionHash }) {
  return (
    <div className="fp-chip">
      <span className="fp-chip-dot"></span>
      <span className="fp-chip-name">{project}</span>
      {sessionHash ? (
        <>
          <span className="fp-chip-sep"></span>
          <span className="fp-chip-hash">{sessionHash}</span>
        </>
      ) : null}
    </div>
  );
}

function DragGrip() {
  return (
    <div className="fp-grip" aria-hidden>
      {Array.from({ length: 8 }).map((_, i) => (
        <span key={i} className="fp-grip-dot"></span>
      ))}
    </div>
  );
}

function QueueBadge({ count }) {
  if (!count || count < 2) return <span style={{ justifySelf: 'end' }}></span>;
  return <div className="fp-queue" title={`${count} queued`}>{count}</div>;
}

function Option({ idx, label, mode, hover, focus, checked }) {
  const cls = [
    'fp-opt',
    mode === 'approve' ? 'fp-opt-approve' : '',
    hover ? 'is-hover' : '',
    focus ? 'is-focus' : '',
    checked ? 'is-checked' : '',
  ].filter(Boolean).join(' ');
  return (
    <button className={cls} tabIndex={-1}>
      {mode === 'multi' && (
        <span className="fp-opt-check" aria-checked={checked}>
          {checked ? <Check /> : null}
        </span>
      )}
      {(mode === 'single' || mode === 'preview') && (
        <span className="fp-opt-num">{idx + 1}.</span>
      )}
      <span className="fp-opt-label">{label}</span>
    </button>
  );
}

function FreeTextInput({ placeholder, value, focus }) {
  return (
    <div className="fp-input-wrap">
      <input
        className={'fp-input ' + (focus ? 'is-focus' : '')}
        placeholder={placeholder}
        defaultValue={value || ''}
        tabIndex={-1}
        readOnly
      />
    </div>
  );
}

// Esc · Esc · Dismiss control. State: 'rest' | 'armed' | 'done' | 'timeout'.
// `progress` is 0..1 for the draining indicator when armed.
function DismissControl({ state = 'rest', progress = 1 }) {
  const pip1Class = (state === 'armed' || state === 'done') ? 'fp-pip is-armed' : 'fp-pip';
  const pip2Class = state === 'done' ? 'fp-pip is-done' : 'fp-pip';
  const showProgress = state === 'armed' || state === 'timeout';
  // progress: armed = drains from full; timeout shows nearly empty
  const scaleX = state === 'armed' ? progress : state === 'timeout' ? 0.04 : 0;
  return (
    <button className="fp-dismiss" tabIndex={-1}>
      <div className="fp-dismiss-pipswrap">
        <span className="fp-dismiss-pips">
          <span className={pip1Class}>Esc</span>
          <span className={pip2Class}>Esc</span>
        </span>
        {showProgress && (
          <span className="fp-dismiss-progress">
            <i style={{ transform: `scaleX(${scaleX})` }}></i>
          </span>
        )}
      </div>
      <span className="fp-dismiss-label">Dismiss</span>
    </button>
  );
}

// ---------- the Popup ----------
function Popup({
  palette,
  session = { project: 'claude-integration', hash: null },
  queue = 1,
  message,         // string or React node
  options = null,  // { mode: 'single'|'multi'|'preview'|'approve', items: [...], hoverIdx, focusIdx, checked: [...], previews: [...], previewIdx }
  input = { placeholder: 'Reply to continue, or double-Esc to let Claude stop.', value: '', focus: false },
  dismiss = { state: 'rest', progress: 1 },
  panelSize = 'auto',   // 'short' | 'auto' | 'tall'
}) {
  const vars = paletteVars(palette);
  const panelClass = panelSize === 'tall'
    ? 'fp-panel fp-panel-tall'
    : panelSize === 'short'
      ? 'fp-panel fp-panel-short'
      : 'fp-panel';

  const hasOptions = options && options.items && options.items.length > 0;
  const isPreview = hasOptions && options.mode === 'preview';

  return (
    <div className="fp-root" style={vars}>
      <div className="fp-top">
        <SessionChip project={session.project} sessionHash={session.hash} />
        <DragGrip />
        <QueueBadge count={queue} />
      </div>

      <div className={panelClass}>{message}</div>

      {hasOptions && !isPreview && (
        <div className="fp-opts">
          {options.items.map((label, i) => (
            <Option
              key={i}
              idx={i}
              label={label}
              mode={options.mode}
              hover={options.hoverIdx === i}
              focus={options.focusIdx === i}
              checked={options.checked && options.checked.includes(i)}
            />
          ))}
        </div>
      )}

      {hasOptions && isPreview && (
        <div className="fp-opts-preview">
          <div className="fp-opts">
            {options.items.map((label, i) => (
              <Option
                key={i}
                idx={i}
                label={label}
                mode="preview"
                hover={options.hoverIdx === i}
                focus={options.focusIdx === i}
              />
            ))}
          </div>
          <div className="fp-preview">
            <div className="fp-preview-label">Preview · option {(options.focusIdx ?? 0) + 1}</div>
            <div>{options.previews[options.focusIdx ?? 0]}</div>
          </div>
        </div>
      )}

      <FreeTextInput {...input} />

      <div className="fp-foot">
        <span>
          <kbd>Enter</kbd> <span style={{ opacity: 0.8 }}>to send</span>
        </span>
        <DismissControl state={dismiss.state} progress={dismiss.progress} />
      </div>
    </div>
  );
}

// Export
Object.assign(window, { Popup, FP_W });
