from publicator.scheduling import schedule_data
from publicator.webui.page import render_page

PENDING = [{"uuid": "00000000-0000-4000-8000-000000000001", "path": "/t/a.png",
            "title": "Tickler", "description": "d", "scheduleTs": 1800000000,
            "price": None, "tier": None, "galleries": []}]


def _render(**overrides):
    kwargs = dict(thumb_map={}, candidates=[], pending=[], existing_ts=[],
                  timeline=[], schedules=schedule_data({}), config={},
                  ai_model="m", openrouter_model="o")
    kwargs.update(overrides)
    return render_page(**kwargs)


def test_page_offers_both_models():
    page = _render(ai_model="claude-cli-opus",
                   openrouter_model="openrouter/google/gemini-2.0-flash-exp:free")
    assert 'id="ai-model"' in page
    assert "claude-cli-opus" in page
    assert "openrouter/google/gemini-2.0-flash-exp:free" in page
    assert "__AI_MODEL__" not in page and "__OPENROUTER_MODEL__" not in page


def test_page_renders_config_tiers_galleries():
    page = _render(candidates=["/t/a.png"],
                   config={"tiers": ["gold"], "galleries": ["Art"]})
    assert 'class="f-tier"' in page and ">gold<" in page
    assert 'class="f-gallery"' in page and 'value="Art"' in page


def test_page_renders_schedule_presets():
    config = {"schedule": {"profiles": [
        {"name": "free", "day": "tuesday", "hour": 20, "per_slot": 2},
        {"name": "paid", "day": "friday", "hour": 20, "per_slot": 1}]}}
    page = _render(candidates=["/t/a.png"], config=config,
                   schedules=schedule_data(config["schedule"]))
    assert 'class="f-preset"' in page
    assert '<option value="free">free</option>' in page
    assert '<option value="paid">paid</option>' in page
    assert '"name": "free"' in page and '"name": "paid"' in page
    assert '"slots"' in page
    assert "__SCHEDULES__" not in page and "__EXISTING_TS__" not in page
    assert "__LABELS__" not in page


def test_pending_card_renders_with_its_schedule_and_uuid():
    page = _render(pending=PENDING, thumb_map={"/t/a.png": "beef.png"})
    assert "Tickler" in page
    assert '00000000-0000-4000-8000-000000000001' in page   # PENDING payload injected
    assert "__PENDING__" not in page and "__CARDS__" not in page


def test_cards_point_at_the_shared_thumbnail_cache():
    page = _render(candidates=["/t/a.png"], thumb_map={"/t/a.png": "beef.png"})
    assert 'src="/thumbs/beef.png"' in page


def test_page_has_a_gallery_and_a_calendar_tab():
    page = _render()
    assert 'id="tab-gallery"' in page and 'id="tab-calendar"' in page
    assert 'id="gallery-tab"' in page and 'id="calendar-tab"' in page
    assert "__CALENDAR__" not in page


def test_calendar_tab_renders_the_timeline():
    rows = [{"uuid": "u1", "ts": 1767726000, "title": "Haunted Tower",
             "url": "https://da/art/Haunted-Tower", "state": "published_or_scheduled",
             "basename": "art.webp", "sha": "ab" * 64}]
    page = _render(timeline=rows)
    assert "January 2026" in page
    assert 'href="https://da/art/Haunted-Tower"' in page


def test_calendar_links_a_queued_entry_to_its_gallery_card():
    rows = [{"uuid": PENDING[0]["uuid"], "ts": 1800000000, "title": "Tickler",
             "url": None, "state": "unpublished",
             "basename": "a.png", "sha": "cd" * 64}]
    page = _render(pending=PENDING, timeline=rows, thumb_map={"/t/a.png": "beef.png"})
    assert 'href="#pending_0"' in page      # the card id render_page gave that entry


def test_render_page_injects_the_prompt_block_into_a_candidate_card():
    from publicator.webui.page import render_page
    page = render_page(
        thumb_map={"picked/a.webp": "aa.webp"}, candidates=["picked/a.webp"],
        pending=[], existing_ts=[], timeline=[], schedules=[],
        config={}, ai_model="m", openrouter_model="",
        prompt_html={"picked/a.webp": '<details class="prompt exact">X</details>'})
    assert '<details class="prompt exact">X</details>' in page


def test_render_page_without_prompt_html_is_unchanged():
    from publicator.webui.page import render_page
    kwargs = dict(
        thumb_map={"picked/a.webp": "aa.webp"}, candidates=["picked/a.webp"],
        pending=[], existing_ts=[], timeline=[], schedules=[],
        config={}, ai_model="m", openrouter_model="")
    assert render_page(**kwargs) == render_page(**kwargs, prompt_html={})


def test_render_page_renders_the_search_box_and_skip_banner():
    from publicator.webui.page import render_page
    page = render_page(
        thumb_map={}, candidates=[], pending=[], existing_ts=[], timeline=[],
        schedules=[], config={}, ai_model="m", openrouter_model="",
        query="tentacles", skipped=3)
    assert "__PROMPTSEARCH__" not in page, "placeholder left in PAGE_TEMPLATE"
    assert 'name="prompt"' in page and 'value="tentacles"' in page
    assert "3 candidates skipped" in page and "their prompt was never archived" in page


def test_skip_banner_uses_singular_wording_for_one():
    from publicator.webui.page import render_page
    page = render_page(
        thumb_map={}, candidates=[], pending=[], existing_ts=[], timeline=[],
        schedules=[], config={}, ai_model="m", openrouter_model="",
        skipped=1)
    assert "1 candidate skipped" in page and "candidates" not in page


def test_render_page_separates_exact_and_hinted_lineage_results():
    from publicator.webui.page import render_page
    page = render_page(
        thumb_map={"a.webp": "a", "b.webp": "b"}, candidates=["a.webp"],
        maybe_candidates=["b.webp"], pending=[], existing_ts=[], timeline=[],
        schedules=[], config={}, ai_model="m", openrouter_model="")
    assert "possibly from this prompt" in page
    # the two groups must not be merged into one grid
    assert page.index("a.webp") < page.index("possibly from this prompt")
    assert page.index("possibly from this prompt") < page.index("b.webp")


def test_render_page_with_search_disabled_emits_no_search_form():
    """A data dir with no [prompts] section must be fully inert, not just
    prompt-less: rendering the form would let a submit run search() (which
    returns nothing) and empty the gallery with no explanation."""
    page = _render(search_enabled=False)
    assert 'class="promptsearch"' not in page
    assert "__PROMPTSEARCH__" not in page


def test_render_page_with_search_enabled_emits_the_search_form():
    page = _render(search_enabled=True)
    assert 'class="promptsearch"' in page


def test_prompt_text_containing_a_placeholder_survives_as_literal_text():
    """__CARDS__ must be the LAST .replace() call: html.escape does not touch
    underscores, so a prompt file whose text is literally "__PROMPTSEARCH__"
    must not get the real search-form markup spliced into its own <pre>."""
    page = _render(candidates=["picked/a.webp"],
                   thumb_map={"picked/a.webp": "aa.webp"},
                   prompt_html={"picked/a.webp": "<pre>__PROMPTSEARCH__</pre>"})
    assert "<pre>__PROMPTSEARCH__</pre>" in page


def test_render_page_notes_truncated_search_results():
    """A broad needle can match more than SEARCH_LIMIT files; the page must
    say so rather than silently showing a shrunk result set."""
    page = _render(truncated=True)
    assert "narrow your search" in page


def test_render_page_labels_the_exact_results_when_a_lineage_filter_is_active():
    page = _render(candidates=["a.webp"], thumb_map={"a.webp": "a"}, lineage_active=True)
    assert "images from this prompt" in page


def test_render_page_omits_the_exact_label_outside_a_lineage_filter():
    page = _render(candidates=["a.webp"], thumb_map={"a.webp": "a"})
    assert "images from this prompt" not in page


def test_render_page_omits_the_exact_label_when_there_are_no_exact_results():
    """Matches the gating already used for "possibly from this prompt" below
    it: an empty labelled section with no explanation reads as a layout
    glitch, not a deliberate zero-results signal."""
    page = _render(candidates=[], lineage_active=True)
    assert "images from this prompt" not in page
