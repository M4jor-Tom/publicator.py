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
