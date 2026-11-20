# Themes

BriskMarkdown draws documents itself (no web browser inside), so a theme is a small, focused
subset of CSS: the parts that make sense for a reading page. If you know CSS you already know
how to write one.

## Using a theme

1. **View › Theme › Open Themes Folder...** opens your themes folder
   (`%APPDATA%\BriskMarkdown\themes`). The first time, it contains `my-theme.css`, a commented
   example.
2. Every `.css` file in that folder appears in **View › Theme** by its file name.
3. After editing a theme, choose **View › Theme › Reload Current Theme**.

If a rule isn't understood, the status bar says so (for example `unknown selector: section`), and
everything else in the theme still applies.

Built-in themes: **Dark** (the default), **Light** and **Sepia**. Their source is in
[`themes/`](../themes). Every theme starts from [`themes/base.css`](../themes/base.css), so you
only need to write what differs.

## The smallest useful theme

Most of the look comes from palette variables. This is a complete theme:

```css
@import "dark";

:root {
  --paper: #0f1115;
  --ink: #e6e1d6;
  --accent: #e0a458;
  --link: #7fc8f8;
}
```

`@import "dark";` (or `"light"`, `"sepia"`) starts from a built-in theme. Without it you start
from the neutral base.

## Palette variables

Set these in `:root`. The base theme and every built-in theme use them throughout.

| Variable | Used for |
|---|---|
| `--paper` | Page background |
| `--surface`, `--surface-2` | Code blocks, table headers, front matter, the status bar |
| `--rule` | Borders, rules, table grid |
| `--ink`, `--ink-2` | Body text, quote text |
| `--muted`, `--quiet` | Secondary text, list markers, labels |
| `--accent` | Caret, checked tasks, the "important" callout |
| `--link` | Links |
| `--code-bg`, `--code-border` | Inline `code` |
| `--selection` | Selected text |
| `--find`, `--find-current` | Search matches |
| `--ok`, `--warn`, `--error`, `--info` | Callouts: tip, warning, caution, note |
| `--sans`, `--mono`, `--heading` | Font families |
| `--tok-keyword` ... `--tok-variable` | Code colouring (see below) |

You can also define your own variables and use them anywhere with `var(--name, fallback)`.

## Selectors

A selector names one element. Combinators (`a b`, `a > b`) and pseudo-classes are not
supported. Separate several selectors with commas.

| Selector | What it styles |
|---|---|
| `:root`, `body` | The page: background, text colour, font, `font-size`, `line-height`, `max-width` (reading column width), `padding`, `caret-color` |
| `h1` ... `h6` | Headings. `border-bottom` draws a rule under the heading |
| `p` | Paragraphs (`margin` sets the spacing between them) |
| `a` | Links |
| `strong`, `em`, `del` | Bold, italic, strikethrough |
| `code` | Inline code (background box, font, size) |
| `pre` | Code blocks (box, font, padding) |
| `.code-lang` | The language label above a code block |
| `blockquote` | Quotes: `border-left` is the bar, `padding-left` the indent |
| `.callout.note`, `.callout.tip`, `.callout.important`, `.callout.warning`, `.callout.caution` | GitHub-style alerts (`> [!NOTE]`). Their `border-left` colour also colours the title |
| `.callout-title` | The callout's title line |
| `li` | List items: `padding-left` is the indent per level, `margin` the spacing |
| `li::marker` | Bullets and numbers |
| `.task`, `.task-done` | Task checkboxes; text of checked items |
| `table`, `th`, `td` | Tables: `th`/`td` `padding` and `border` set the grid |
| `hr` | Horizontal rules (`border-top`) |
| `.math`, `.math-block` | Inline and display math |
| `.diagram` | Mermaid and other diagram blocks (shown as source) |
| `kbd`, `mark`, `sup` | `<kbd>`, `<mark>`, superscripts and footnote references |
| `.front-matter` | The YAML block at the top of a file |
| `.html` | Text extracted from raw HTML blocks |
| `.footnote` | Footnote definitions |
| `.image`, `.wikilink` | Image placeholders and `[[wiki links]]` |
| `.mono` | Mono mode: font, size, colours, padding |
| `::selection` | Selection highlight (`background`) |
| `.find`, `.find-current` | Search highlights (`background`) |
| `.menubar` | The menu bar (`background`, `color`) |
| `.status`, `.status-button` | The status bar and its Markdown/Mono switch |
| `.scrollbar` | The scrollbar thumb (`background`) |
| `.tok-keyword`, `.tok-string`, `.tok-comment`, `.tok-number`, `.tok-type`, `.tok-function`, `.tok-constant`, `.tok-property`, `.tok-tag`, `.tok-meta`, `.tok-inserted`, `.tok-deleted`, `.tok-variable` | Code colouring |

## Properties

| Property | Values |
|---|---|
| `color`, `background` / `background-color` | `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb()`, `rgba()`, `hsl()`, `color-mix(in srgb, a 40%, b)`, a few names, `transparent` |
| `font-family` | A list; the first installed font wins. `sans-serif`, `serif`, `monospace` map to Segoe UI, Georgia and Consolas |
| `font-size` | `px`, `em`, `rem`, `%`, `pt` |
| `font-weight` | `normal`, `bold`, `100` to `900` |
| `font-style` | `normal`, `italic` |
| `font` | Shorthand: `[italic] [weight] size[/line-height] family` |
| `line-height` | A number (multiple of the font size) or a length |
| `margin`, `margin-top`, `margin-bottom` | Lengths; vertical margins between blocks collapse as in CSS |
| `padding`, `padding-*` | Lengths |
| `border`, `border-top`, `border-right`, `border-bottom`, `border-left` | `width style color`, for example `1px solid #343a43` |
| `border-color`, `border-left-color` | A colour |
| `border-radius` | A length |
| `max-width` | On `body`: the reading column width |
| `text-decoration` | `underline`, `line-through`, `none` |
| `caret-color` | On `body`: the caret |

Common web properties that don't apply here (`display`, `overflow`, `white-space`,
`box-shadow`, ...) are accepted and ignored, so you can paste rules from a web stylesheet.
