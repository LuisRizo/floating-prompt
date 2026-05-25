// Floating-Prompt — canvas content (artboards)
// Lives in floating-prompt.html. Imports Popup from popup.jsx and
// FP_PALETTES from palettes.js. Builds all the artboards specified
// in §7 of the brief.

const P = window.FP_PALETTES;
const SLATE = P.slate;

// ---------- sample message bodies ----------
const M_STOP_SHORT = (
  <p>
    Done. Migrated <code>14</code> files and updated the snapshot tests.
    Two of the snapshots changed in ways worth eyeballing —
    <code> ProfileMenu</code> and <code>SessionBadge</code>.
  </p>
);

const M_STOP_LONG = (
  <>
    <p>
      I pulled the auth flow apart and re-built it around the new
      <code> RefreshTokenLease</code> primitive. Summary of what changed:
    </p>
    <p>
      <strong>Server</strong> — the legacy <code>/oauth/refresh</code> handler
      now delegates to <code>LeaseStore.acquire()</code> instead of writing
      directly to the cache. That lets a second concurrent request piggy-back
      on the same upstream refresh rather than firing its own. Tests for the
      lease race are in <code>auth/lease.spec.ts</code> — all passing.
    </p>
    <p>
      <strong>Client</strong> — replaced the imperative <code>refresh()</code>
      call with a React Query mutation, so the staleness check now happens
      in the query cache instead of being scattered across each call site.
      I removed <code>useAuthRefresh</code> entirely; the four places that
      called it now read from <code>useSession()</code>.
    </p>
    <p>
      <strong>Migration note</strong> — three feature flags referenced the
      old code path; I left them in place but flipped them to no-ops so a
      rollback can flip them back without code changes. They should be deleted
      next sprint.
    </p>
    <p>
      <strong>Open question</strong> — the cookie domain logic in
      <code> cookieGuard.ts</code> still hard-codes the production host. I
      didn't touch it because the existing tests would have needed a rewrite,
      but it's the obvious next thing to fix.
    </p>
    <p>
      Ready for review. Want me to open the PR or hold while you look?
    </p>
  </>
);

const M_PLAN = (
  <>
    <p>
      Here's the plan for the dashboard refactor. I'll wait for approval
      before touching anything.
    </p>
    <p>
      <strong>1 · Extract chart primitives.</strong> Pull <code>LineChart</code>,
      <code> BarChart</code>, <code>SparkChart</code> out of
      <code> dashboard/widgets/</code> into a new <code>charts/</code> package.
      They currently re-implement axes three different ways; one shared
      <code> Axis</code> component will replace all three.
    </p>
    <p>
      <strong>2 · Lift the data layer.</strong> The widgets each fetch their
      own data with bespoke <code>useEffect</code> calls. I'll replace them
      with a single <code>useDashboardData()</code> hook backed by React Query,
      so the dashboard gets one coordinated refresh instead of nine
      independent ones.
    </p>
    <p>
      <strong>3 · Consolidate the date-range picker.</strong> The picker
      currently lives inside <code>OverviewWidget</code>; I'll hoist it to
      <code> DashboardShell</code> so it controls every widget at once. The
      Storybook story for the picker stays as-is.
    </p>
    <p>
      <strong>4 · Storybook + tests.</strong> Each new chart primitive gets
      a story file and a visual-regression test. I'll add a smoke test for
      the dashboard that mounts every widget against a deterministic fixture.
    </p>
    <p>
      Estimated diff: ~1,800 lines added, ~1,400 removed. No public API
      changes. Should I proceed?
    </p>
  </>
);

const M_Q_SHORT_3 = (
  <p>
    Which approach for handling tokens that expire mid-request?
  </p>
);
const Q_SHORT_3_OPTS = [
  'Refresh on read',
  'Refresh on a schedule',
  'Defer until the next request',
];

const M_Q_LONG_3 = (
  <>
    <p>
      The migration script left orphaned rows in three tables —
      <code> user_sessions</code>, <code>device_grants</code>, and
      <code> audit_log</code> — when it bailed on the failed batch. Which
      cleanup approach should I take?
    </p>
  </>
);
const Q_LONG_3_OPTS = [
  'Roll back the whole migration and retry from scratch.',
  'Run the cleanup script in dry-run mode first, then apply.',
  'Leave the orphans for the nightly GC job to pick up.',
];

const M_MULTI = (
  <p>
    Which of these should I run before opening the PR? Pick any.
  </p>
);
const MULTI_OPTS = [
  'Re-run the affected snapshot tests',
  'Regenerate the OpenAPI types',
  'Bump the changelog for the public package',
  'Format the diff with the repo prettier config',
];

const M_PREVIEW = (
  <p>
    Which diff style for the auto-generated changelog?
  </p>
);
const PREVIEW_OPTS = [
  'Conventional (grouped by type)',
  'Per-file unified diff',
  'PR-style narrative',
];
const PREVIEW_BODIES = [
`## v1.42.0

### Features
- charts: add SparkChart primitive
- auth: lease-based refresh tokens

### Fixes
- session: handle stale cookies on
  cross-subdomain navigation

### Internal
- deps: bump react-query → 5.4`,
` src/charts/index.ts          | +14 -0
 src/charts/SparkChart.tsx    | +88 -0
 src/auth/lease.ts            | +52 -3
 src/auth/cookieGuard.ts      |  +4 -2
 src/session/useSession.ts    | +12 -9
 src/widgets/Overview.tsx     | +18 -41
 ────────────────────────────────────
 7 files changed, +188 -55`,
`The 1.42 release reworks how
the client refreshes auth tokens
and introduces a small charts
package extracted from the
dashboard widgets.

The auth change is non-breaking;
the charts move is internal
but consumers should reach for
\`@app/charts\` going forward.`,
];

// ---------- Artboard helper ----------
function Stage({ children, width = 580, padTop = 36 }) {
  // Centers the popup inside the artboard with a quiet "wallpaper" tone
  // so the popup's shadow has somewhere to fall — mimics what the user
  // sees when the popup appears on top of their desktop.
  return (
    <div style={{
      width: '100%',
      height: '100%',
      background: 'linear-gradient(180deg, #0a0a0c 0%, #111114 100%)',
      display: 'flex',
      justifyContent: 'center',
      alignItems: 'flex-start',
      paddingTop: padTop,
      paddingBottom: 36,
      boxSizing: 'border-box',
    }}>
      {children}
    </div>
  );
}

// Wraps an artboard's popup so the height auto-fits content.
function Card(props) {
  return (
    <Stage>
      <Popup {...props} />
    </Stage>
  );
}

// ---------- mockup artboards ----------
function ArtStopShort() {
  return <Card palette={SLATE} session={{ project: 'claude-integration' }} queue={1}
    message={M_STOP_SHORT}
    panelSize="short"
    input={{ placeholder: 'Reply to continue, or double-Esc to let Claude stop.' }}
  />;
}

function ArtStopLong() {
  return <Card palette={SLATE} session={{ project: 'claude-integration' }} queue={1}
    message={M_STOP_LONG}
    panelSize="auto"
    input={{ placeholder: 'Reply to continue, or double-Esc to let Claude stop.' }}
  />;
}

function ArtQ3Short() {
  return <Card palette={SLATE} session={{ project: 'auth-service' }} queue={1}
    message={M_Q_SHORT_3}
    panelSize="short"
    options={{ mode: 'single', items: Q_SHORT_3_OPTS, focusIdx: 0 }}
    input={{ placeholder: 'Type a custom answer…' }}
  />;
}

function ArtQ3LongQueued() {
  return <Card palette={SLATE} session={{ project: 'backend-api' }} queue={3}
    message={M_Q_LONG_3}
    panelSize="short"
    options={{ mode: 'single', items: Q_LONG_3_OPTS, hoverIdx: 1 }}
    input={{ placeholder: 'Type a custom answer…' }}
  />;
}

function ArtMulti4() {
  return <Card palette={SLATE} session={{ project: 'claude-integration' }} queue={1}
    message={M_MULTI}
    panelSize="short"
    options={{ mode: 'multi', items: MULTI_OPTS, checked: [0, 3] }}
    input={{ placeholder: 'Or type a custom answer…' }}
  />;
}

function ArtPreview() {
  return <Card palette={SLATE} session={{ project: 'docs-site' }} queue={1}
    message={M_PREVIEW}
    panelSize="short"
    options={{ mode: 'preview', items: PREVIEW_OPTS, focusIdx: 0, previews: PREVIEW_BODIES }}
    input={{ placeholder: 'Type a custom answer…' }}
  />;
}

function ArtPlan() {
  return <Card palette={SLATE} session={{ project: 'dashboard-refactor' }} queue={1}
    message={M_PLAN}
    panelSize="auto"
    options={{ mode: 'approve', items: ['Approve'] }}
    input={{ placeholder: 'Or describe changes to the plan…' }}
  />;
}

// ---------- palette family ----------
function ArtPalette({ palette }) {
  return <Card palette={palette}
    session={{ project: palette.name + '-project' }}
    queue={1}
    message={
      <p>
        Pulled the spec apart and re-implemented the lease handshake. All
        tests pass; want me to open the PR?
      </p>
    }
    panelSize="short"
    options={{ mode: 'single', items: ['Open the PR now', 'Hold for review', 'Run integration tests first'], focusIdx: 0 }}
    input={{ placeholder: 'Type a custom answer…' }}
  />;
}

// ---------- interaction states ----------
function ArtOptHover() {
  return <Card palette={SLATE} session={{ project: 'auth-service' }} queue={1}
    message={<p>Which approach for handling tokens that expire mid-request?</p>}
    panelSize="short"
    options={{ mode: 'single', items: Q_SHORT_3_OPTS, hoverIdx: 1 }}
    input={{ placeholder: 'Type a custom answer…' }}
  />;
}

function ArtOptFocus() {
  return <Card palette={SLATE} session={{ project: 'auth-service' }} queue={1}
    message={<p>Which approach for handling tokens that expire mid-request?</p>}
    panelSize="short"
    options={{ mode: 'single', items: Q_SHORT_3_OPTS, focusIdx: 1 }}
    input={{ placeholder: 'Type a custom answer…' }}
  />;
}

function ArtInputEmpty() {
  return <Card palette={SLATE} session={{ project: 'claude-integration' }} queue={1}
    message={M_STOP_SHORT}
    panelSize="short"
    input={{ placeholder: 'Reply to continue, or double-Esc to let Claude stop.', focus: false }}
  />;
}

function ArtInputFilled() {
  return <Card palette={SLATE} session={{ project: 'claude-integration' }} queue={1}
    message={M_STOP_SHORT}
    panelSize="short"
    input={{
      placeholder: 'Reply to continue, or double-Esc to let Claude stop.',
      value: "Open the PR but mark it as draft, I'll review it after lunch.",
      focus: true,
    }}
  />;
}

// ---------- drag-affordance close-up ----------
function ArtDragAffordance() {
  // A magnified vignette of the upper chrome so the grip dots read.
  return (
    <Stage padTop={60}>
      <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 28 }}>
        <div style={{ transform: 'scale(1.55)', transformOrigin: 'top center' }}>
          <div className="fp-root" style={{
            ...paletteVarsHelper(SLATE),
            paddingBottom: 0,
            boxShadow: '0 24px 60px -12px rgba(0,0,0,0.55), 0 8px 24px -8px rgba(0,0,0,0.4), 0 0 0 1px rgba(255,255,255,0.06)',
          }}>
            <div className="fp-top">
              <div className="fp-chip">
                <span className="fp-chip-dot"></span>
                <span className="fp-chip-name">claude-integration</span>
              </div>
              <div className="fp-grip" style={{ opacity: 0.72 }}>
                {Array.from({ length: 8 }).map((_, i) => <span key={i} className="fp-grip-dot"></span>)}
              </div>
              <div className="fp-queue">2</div>
            </div>
            <div style={{
              height: 56,
              background: SLATE.panel,
              borderRadius: 10,
              opacity: 0.6,
            }}></div>
          </div>
        </div>
        <div style={{
          maxWidth: 380,
          fontFamily: 'Geist, sans-serif',
          color: '#a8acb4',
          fontSize: 12.5,
          lineHeight: 1.5,
          textAlign: 'center',
        }}>
          The entire top row is the drag region. Four faint dots in the
          center read as a handle; they brighten as the cursor enters the
          row, and the cursor swaps to <span style={{ color: '#d6dae0' }}>grab</span>.
          The message body below is not draggable so text stays selectable.
        </div>
      </div>
    </Stage>
  );
}
// the popup CSS-var helper duplicated locally because paletteVars lives
// inside popup.jsx's module scope — we re-derive here.
function paletteVarsHelper(p) {
  return {
    '--fp-bg': p.bg, '--fp-panel': p.panel, '--fp-chip': p.chip,
    '--fp-chip-border': p.chipBorder, '--fp-accent': p.accent,
    '--fp-accent-soft': p.accentSoft, '--fp-opt-bg': p.optionBg,
    '--fp-opt-hover': p.optionHover, '--fp-opt-border': p.optionBorder,
    '--fp-opt-num': p.optionNumber, '--fp-input-bg': p.inputBg,
    '--fp-input-border': p.inputBorder, '--fp-body': p.body,
    '--fp-title': p.title, '--fp-dim': p.dim, '--fp-scroll': p.scrollThumb,
  };
}

// ---------- dismiss-control filmstrip ----------
function FilmCell({ label, state, progress }) {
  return (
    <div style={{
      display: 'flex',
      flexDirection: 'column',
      gap: 14,
      alignItems: 'flex-start',
    }}>
      <div style={{
        fontFamily: 'Geist Mono, monospace',
        fontSize: 10.5,
        letterSpacing: 0.08,
        textTransform: 'uppercase',
        color: '#7a818c',
      }}>{label}</div>
      <div style={{
        ...paletteVarsHelper(SLATE),
        background: SLATE.bg,
        borderRadius: 12,
        padding: '14px 16px',
        border: '1px solid rgba(255,255,255,0.06)',
        boxShadow: '0 12px 32px -12px rgba(0,0,0,0.6)',
        width: 260,
      }}>
        <div className="fp-foot">
          <span style={{ color: SLATE.dim }}>
            <kbd>Enter</kbd> <span style={{ opacity: 0.8 }}>to send</span>
          </span>
          <DismissControlInline state={state} progress={progress} />
        </div>
      </div>
    </div>
  );
}

function DismissControlInline({ state, progress }) {
  // Recreate the dismiss control here so we don't need to import it.
  const pip1Class = (state === 'armed' || state === 'done') ? 'fp-pip is-armed' : 'fp-pip';
  const pip2Class = state === 'done' ? 'fp-pip is-done' : 'fp-pip';
  const showProgress = state === 'armed' || state === 'timeout';
  const scaleX = state === 'armed' ? progress : state === 'timeout' ? 0.04 : 0;
  return (
    <span className="fp-dismiss">
      <span className="fp-dismiss-pipswrap">
        <span className="fp-dismiss-pips">
          <span className={pip1Class}>Esc</span>
          <span className={pip2Class}>Esc</span>
        </span>
        {showProgress && (
          <span className="fp-dismiss-progress">
            <i style={{ transform: `scaleX(${scaleX})` }}></i>
          </span>
        )}
      </span>
      <span className="fp-dismiss-label">Dismiss</span>
    </span>
  );
}

function ArtFilmstrip() {
  return (
    <Stage padTop={48}>
      <div style={{
        display: 'grid',
        gridTemplateColumns: 'repeat(2, auto)',
        gap: '36px 28px',
      }}>
        <FilmCell label="01 · Resting"          state="rest"    progress={1}    />
        <FilmCell label="02 · First Esc · armed" state="armed"   progress={0.62} />
        <FilmCell label="03 · Second Esc · done" state="done"    progress={0}    />
        <FilmCell label="04 · Timed out"         state="timeout" progress={0}    />
      </div>
    </Stage>
  );
}

// Expose
Object.assign(window, {
  ArtStopShort, ArtStopLong, ArtQ3Short, ArtQ3LongQueued, ArtMulti4,
  ArtPreview, ArtPlan, ArtPalette, ArtOptHover, ArtOptFocus,
  ArtInputEmpty, ArtInputFilled, ArtDragAffordance, ArtFilmstrip,
});
