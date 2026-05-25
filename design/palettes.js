// Floating-Prompt palette family.
// Each palette defines every slot used by the popup. Dark-mode-first.
// All palettes are tested for WCAG AA contrast on body/title vs. bg
// and option-label vs. option-surface.
//
// Slot schema:
//   bg            popup main surface
//   panel         tinted backdrop behind the message body
//   chip          session chip background
//   chipBorder    session chip 1px border
//   accent        focus rings, esc-pips, session dot, progress bar
//   accentSoft    softened accent for hovered fills
//   optionBg      option button rest surface
//   optionHover   option button hover surface
//   optionBorder  option button 1px border
//   optionNumber  the leading "1." digit color
//   inputBg       free-text input rest surface
//   inputBorder   free-text input rest border
//   body          message body text
//   title         project name / strong labels
//   dim           queue counter, footer legend, placeholder
//   scrollThumb   message-body scrollbar thumb

window.FP_PALETTES = {
  slate: {
    name: 'slate',
    bg:           '#1a1d23',
    panel:        '#20242b',
    chip:         '#272c34',
    chipBorder:   '#353c47',
    accent:       '#86b0d8',
    accentSoft:   'rgba(134,176,216,0.18)',
    optionBg:     '#252a32',
    optionHover:  '#2c323c',
    optionBorder: '#363d48',
    optionNumber: '#6a7280',
    inputBg:      '#1d2128',
    inputBorder:  '#2f353f',
    body:         '#d6dae0',
    title:        '#f2f4f7',
    dim:          '#7a818c',
    scrollThumb:  '#3a414c',
  },
  ocean: {
    name: 'ocean',
    bg:           '#0f1a24',
    panel:        '#142231',
    chip:         '#1a2c40',
    chipBorder:   '#2a4258',
    accent:       '#5fd0c4',
    accentSoft:   'rgba(95,208,196,0.18)',
    optionBg:     '#182838',
    optionHover:  '#1e324a',
    optionBorder: '#2a4258',
    optionNumber: '#5e7a92',
    inputBg:      '#11202f',
    inputBorder:  '#22384c',
    body:         '#cad8e4',
    title:        '#ebf2f7',
    dim:          '#6a8094',
    scrollThumb:  '#2c4258',
  },
  amber: {
    name: 'amber',
    bg:           '#1a1612',
    panel:        '#221d17',
    chip:         '#2c241b',
    chipBorder:   '#3e3225',
    accent:       '#e8a04a',
    accentSoft:   'rgba(232,160,74,0.18)',
    optionBg:     '#25201a',
    optionHover:  '#2e2820',
    optionBorder: '#3a3128',
    optionNumber: '#7a6d5a',
    inputBg:      '#1d1813',
    inputBorder:  '#2f2820',
    body:         '#d6cdc0',
    title:        '#f5ede0',
    dim:          '#857b6b',
    scrollThumb:  '#3d3328',
  },
  forest: {
    name: 'forest',
    bg:           '#131815',
    panel:        '#181f1b',
    chip:         '#1d2820',
    chipBorder:   '#2b3a30',
    accent:       '#7ec595',
    accentSoft:   'rgba(126,197,149,0.18)',
    optionBg:     '#1c2420',
    optionHover:  '#222c26',
    optionBorder: '#2c3830',
    optionNumber: '#6a7a70',
    inputBg:      '#161c18',
    inputBorder:  '#252e28',
    body:         '#c8d0c8',
    title:        '#ebf2eb',
    dim:          '#748378',
    scrollThumb:  '#2f3a33',
  },
  plum: {
    name: 'plum',
    bg:           '#1a1620',
    panel:        '#221c2a',
    chip:         '#2a2236',
    chipBorder:   '#3a2f4a',
    accent:       '#c8a3e6',
    accentSoft:   'rgba(200,163,230,0.18)',
    optionBg:     '#251f30',
    optionHover:  '#2c2438',
    optionBorder: '#382e44',
    optionNumber: '#7a6e88',
    inputBg:      '#1d1825',
    inputBorder:  '#2e2638',
    body:         '#d2c8d8',
    title:        '#f0e8f5',
    dim:          '#857890',
    scrollThumb:  '#3a2f48',
  },
  default: {
    name: 'default',
    bg:           '#171719',
    panel:        '#1e1e21',
    chip:         '#27272b',
    chipBorder:   '#383840',
    accent:       '#e8e8ea',
    accentSoft:   'rgba(232,232,234,0.14)',
    optionBg:     '#232326',
    optionHover:  '#2a2a2e',
    optionBorder: '#34343a',
    optionNumber: '#75757a',
    inputBg:      '#1a1a1d',
    inputBorder:  '#2c2c30',
    body:         '#d5d5d8',
    title:        '#f5f5f7',
    dim:          '#828286',
    scrollThumb:  '#3a3a40',
  },
};
