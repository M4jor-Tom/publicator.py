"""The prompt block on a gallery card: an image's generation prompt, or an
explicitly labelled near-miss.

Pure, like calendar_view: it is handed a resolved match and returns HTML.

The visual split is the point. An Exact reads as the prompt; a Nearest reads as
a warning that happens to contain other people's prompts. The type makes the
mistake impossible in code (Nearest has no .text); this module makes it hard to
make by eye."""

import html
from datetime import datetime

from publicator.prompts import Exact, Nearest, PromptMatch, rank_paths


def _stamp(committed: int) -> str:
    """Blobs recovered from unreferenced objects have no commit to date them."""
    if not committed:
        return "unknown date"
    return datetime.fromtimestamp(committed).strftime("%Y-%m-%d")


def _body(text: str) -> str:
    """Raw <pre>, for plain-text and JSON prompts alike — the JSON ones are
    already pretty-printed on disk, so parsing them buys nothing."""
    return f"<pre>{html.escape(text)}</pre>"


def render_prompt(match: PromptMatch, near: str = "") -> str:
    if isinstance(match, Exact):
        v = match.version
        paths = " · ".join(html.escape(p) for p in rank_paths(v.paths, near))
        return (f'<details class="prompt exact"><summary>prompt · '
                f'{paths or "path unknown"} · <code>{html.escape(v.version[:7])}</code>'
                f'</summary>{_body(v.text)}</details>')
    if isinstance(match, Nearest):
        items = []
        for lin in match.candidates:
            n = len(lin.versions)
            versions = "".join(
                f'<details class="pv"><summary>{_stamp(v.committed)} · '
                f'<code>{html.escape(v.version[:7])}</code></summary>'
                f"{_body(v.text)}</details>" for v in lin.versions)
            items.append(
                f'<li><span class="lin">{html.escape(lin.path)}</span> '
                f'<span class="n">{n} known version{"" if n == 1 else "s"}</span>'
                f"{versions}</li>")
        return ('<details class="prompt nearest">'
                "<summary>prompt NOT ARCHIVED ⚠</summary>"
                '<p class="warn">This exact prompt was never committed and is '
                "unrecoverable. Other versions of the same prompt file — "
                "<strong>NOT</strong> what made this image:</p>"
                f'<ul>{"".join(items)}</ul></details>')
    return ""
