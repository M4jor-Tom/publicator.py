import re

from publicator.scheduling import schedule_data
from publicator.webui.page import render_page


def _render(**overrides):
    kwargs = dict(thumb_map={}, candidates=[], publications=[], timeline=[],
                  schedules=schedule_data({}), config={}, ai_model="m", openrouter_model="o",
                  platforms={"deviantart": "DeviantArt"})
    kwargs.update(overrides)
    return render_page(**kwargs)


def publication(state="SCHEDULED", **fields):
    return {"uuid": "00000000-0000-4000-8000-000000000001", "state": state,
            "path": "/t/a.png", "title": "Manual", "description": "draft",
            "apparitionTimestampIfDifferentThanSubmission": 1800000000,
            "apparitions": [], **fields}


def test_four_completed_phase_counts_and_native_inputs():
    page = _render(candidates=["/t/c.png"], publications=[publication("PICKED"),
                   publication(), publication("DESCRIBED"), publication("APPEARED")])
    for phase, count in [("pick", 1), ("schedule", 1), ("describe", 1), ("publish", 2)]:
        assert f'id="tab-{phase}"' in page and f'id="{phase}-tab"' in page
        assert f'{phase.capitalize()} ({count})' in page
    assert 'type="datetime-local"' in page and 'data-field="title"' in page
    assert 'new Date(' not in page and 'const queue' not in page


def test_candidate_has_pick_prompt_view_delete_but_no_metadata():
    page = _render(candidates=["/t/c.png"], thumb_map={"/t/c.png": "beef.png"},
                   publications=[publication()], prompt_html={"/t/c.png": "<pre>candidate prompt</pre>",
                                                              "/t/a.png": "<pre>not on draft</pre>"})
    candidate = re.search(r'<article class="card candidate".*?</article>', page, re.S).group()
    assert 'data-action="pick"' in candidate and 'data-action="delete"' in candidate
    assert '/original?path=' in candidate and 'candidate prompt' in candidate
    assert 'data-field=' not in candidate and 'not on draft' not in page
    assert 'src="/thumbs/beef.png"' in candidate


def test_drafts_escaped_and_markers_visible():
    page = _render(publications=[publication(title='</script><script>alert(1)</script>',
                   description='[[PUBLICATOR:TODO_GENERATE]]')])
    assert '</script><script>alert(1)</script>' not in page
    assert '&lt;/script&gt;' in page and 'AI metadata pending' in page
    assert 'beforeunload' in page and 'await flushDrafts()' in page
    assert '.textContent' in page and 'innerHTML' not in page


def test_platform_table_accessible_locked_success_and_options():
    page = _render(publications=[publication('APPEARED', apparitions=[{
        'platformName': 'deviantart', 'state': 'published_or_scheduled'}]),
        publication('DESCRIBED', uuid='00000000-0000-4000-8000-000000000002', apparitions=[{
        'platformName': 'deviantart', 'state': 'unpublished', 'priceIfNotFree': 4,
        'tiers': ['gold'], 'galleries': ['Art']}])], config={'tiers': ['gold'], 'galleries': ['Art']})
    assert '<table' in page and 'scope="col"' in page and 'scope="row"' in page
    assert 'Published / scheduled' in page and 'checked disabled' in page
    assert 'id="select-all"' in page and '.indeterminate' in page
    assert 'aria-label="Manual on DeviantArt"' in page
    assert 'value="4"' in page and 'value="gold" checked' in page and 'value="Art" checked' in page
    assert 'Publish selected' in page and 'Retry' in page


def test_platform_options_do_not_select_obsolete_singular_tier():
    page = _render(publications=[publication('DESCRIBED', apparitions=[{
        'platformName': 'deviantart', 'state': 'unpublished', 'tier': 'gold'}])],
        config={'tiers': ['gold']})
    assert 'value="gold"' in page
    assert 'value="gold" checked' not in page


def test_ai_catalog_and_requested_fields_are_in_describe_only():
    from publicator.llm_meta import VISION_MODELS
    page = _render(publications=[publication()], vision_models=VISION_MODELS,
        ai_model='claude-custom', openrouter_model=VISION_MODELS[0]['openrouter_name'],
        local_model_statuses={VISION_MODELS[0]['ollama_name']: {
            'fit': True, 'available': True, 'installed': False}})
    assert page.count('data-source="openrouter"') == 1
    assert page.count('data-source="local"') == len(VISION_MODELS)
    assert 'Install on use' in page and 'claude-custom' in page
    assert 'data-generate=' not in page and 'Fields to generate' not in page
    assert 'body:JSON.stringify({uuid, source:opt.dataset.source, model:' in page
    assert 'JSON.stringify({uuid, source:opt.dataset.source, model:opt.dataset.model ? JSON.parse(opt.dataset.model) : null, fields' not in page
    assert 'Mark for later' not in page


def test_schedule_warnings_and_no_slot_approval_block():
    page = _render(publications=[publication('PICKED', apparitionTimestampIfDifferentThanSubmission=None)])
    assert 'No slot available' in page and 'data-action="approve-date" disabled' in page
    assert 'Use suggested date' in page


def test_render_page_renders_the_search_box_and_skip_banner():
    from publicator.webui.page import render_page
    page = render_page(
        thumb_map={}, candidates=[], publications=[], timeline=[],
        schedules=[], config={}, ai_model="m", openrouter_model="", platforms={},
        query="tentacles", skipped=3)
    assert "__PROMPTSEARCH__" not in page, "placeholder left in PAGE_TEMPLATE"
    assert 'name="prompt"' in page and 'value="tentacles"' in page
    assert "3 candidates skipped" in page and "their prompt was never archived" in page


def test_skip_banner_uses_singular_wording_for_one():
    from publicator.webui.page import render_page
    page = render_page(
        thumb_map={}, candidates=[], publications=[], timeline=[],
        schedules=[], config={}, ai_model="m", openrouter_model="", platforms={},
        skipped=1)
    assert "1 candidate skipped" in page and "candidates" not in page


def test_render_page_separates_exact_and_hinted_lineage_results():
    from publicator.webui.page import render_page
    page = render_page(
        thumb_map={"a.webp": "a", "b.webp": "b"}, candidates=["a.webp"],
        maybe_candidates=["b.webp"], publications=[], timeline=[],
        schedules=[], config={}, ai_model="m", openrouter_model="", platforms={})
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


def test_pick_search_controls_render_limit_paths_and_reload():
    from publicator.images import Search
    page = _render(search=Search(50, ('picked',)), publicable_choices=['picked', 'other'])
    assert all(f'<option value="{limit}"' in page for limit in (0, 25, 50, 75, 100))
    assert '<option value="50" selected' in page
    assert 'type="checkbox" name="search-path" value="picked" checked' in page
    assert 'type="checkbox" name="search-path" value="other"' in page
    assert '>Reload</button>' in page
    assert "post('/search', {limit, paths})" in page


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


def test_native_controls_disable_firefox_stale_form_restoration():
    from html.parser import HTMLParser
    class Controls(HTMLParser):
        def handle_starttag(self, tag, attrs):
            if tag in ('input', 'select', 'textarea'):
                assert dict(attrs).get('autocomplete') == 'off'
    Controls().feed(_render(candidates=['a.png'], publications=[publication('PICKED'), publication(),
        publication('DESCRIBED', apparitions=[{'platformName': 'deviantart', 'state': 'unpublished'}])]))


def test_removed_schedule_preset_is_displayed_as_custom():
    page = _render(publications=[publication('PICKED', scheduleProfile='removed', scheduleIsManual=True)])
    assert '<option value="" selected disabled>Custom / unassigned</option>' in page
    assert 'Manual pin' in page
