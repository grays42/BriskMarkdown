---
title: BriskMarkdown showcase
tags: [markdown, demo]
---

# BriskMarkdown

A **fast**, *lightweight* Markdown notepad for Windows. It opens in a blink, renders the way you
like to read, and lets you edit in place. Inline `code`, ~~strikethrough~~, [links](https://example.com),
and a footnote[^1] all render cleanly. Emoji too: ✅ 🚀 📝.

## What it renders

- Headings, paragraphs, **bold**, *italic*, ***both***
- Nested lists
  - with bullets that change by depth
    - like this
- Task lists:
  - [x] Open instantly
  - [ ] Read everything an AI writes
1. Ordered lists
2. Keep their numbers
3. Even past nine

> A block quote keeps a quiet bar on its left,
> and wraps long lines without fuss.

> [!NOTE]
> GitHub-style callouts are supported: note, tip, important, warning and caution.

> [!WARNING]
> Tables, math and code are edited in Mono mode.

| Feature        | Markdown mode | Mono mode |
|:---------------|:-------------:|----------:|
| Read           | ✓             | ✓         |
| Type prose     | ✓             | ✓         |
| Edit tables    | —             | ✓         |

```rust
fn main() {
    let greeting = "Hello, world!"; // say hi
    println!("{greeting} {}", 42);
}
```

```python
@dataclass
class Point:
    x: float = 0.0  # coordinates
    def norm(self) -> float:
        return math.sqrt(self.x ** 2)
```

Inline math like $E = mc^2$ and $\alpha + \beta \le \gamma$, and display math:

$$
\sum_{i=1}^{n} x_i = \frac{n(n+1)}{2}
$$

---

Press <kbd>Ctrl</kbd>+<kbd>/</kbd> to switch to Mono mode.<br>
Line two after an HTML break.

<details>
<summary>More details</summary>

Hidden content renders as text.

</details>

[^1]: Footnotes appear where they are defined.
