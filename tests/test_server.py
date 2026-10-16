import json
import http.client
import threading
import contextlib
from types import SimpleNamespace

import pytest
import urllib.error
import urllib.request
from http.server import ThreadingHTTPServer

from publicator.images import thumb_name
from publicator.webui.server import GalleryHandler, thumb_maps


def test_thumb_maps_pairs_gallery_art_with_calendar_rows(tmp_path):
    picked = tmp_path / "picked"
    picked.mkdir()
    pick = picked / "new.png"
    pick.write_bytes(b"\x89PNG-new")
    old = picked / "old.webp"          # published art, still on disk
    old.write_bytes(b"RIFF-old")
    rows = [{"basename": "old.webp", "sha": "ab" * 64},
            {"basename": "gone.webp", "sha": "cd" * 64}]

    thumb_map, thumb_src = thumb_maps(str(tmp_path), [str(pick)], rows)

    assert thumb_map == {str(pick): thumb_name(str(pick))}       # cards -> cache names
    assert thumb_src[thumb_name(str(pick))] == str(pick)         # and back, for serving
    assert thumb_src["ab" * 16 + ".webp"] == str(old)            # stored sha, no re-hash
    assert "cd" * 16 + ".webp" not in thumb_src                  # art no longer on disk
    assert rows[0]["thumb"] == "ab" * 16 + ".webp"               # stamped for the view
    assert rows[1]["thumb"] == ""                                # ...which shows text instead
    assert rows[0]["path"] == str(old)   # how the calendar looks its prompt up
    assert rows[1]["path"] == ""         # no file, so no prompt to resolve


def test_thumb_maps_hashes_a_row_the_json_never_recorded_a_sha_for(tmp_path):
    art = tmp_path / "old.webp"
    art.write_bytes(b"RIFF-old")
    rows = [{"basename": "old.webp", "sha": ""}]

    _, thumb_src = thumb_maps(str(tmp_path), [], rows)

    assert rows[0]["thumb"] == thumb_name(str(art))   # hashed from the file it found
    assert thumb_src[rows[0]["thumb"]] == str(art)


def _serve_once(**attrs):
    for k, v in attrs.items():
        setattr(GalleryHandler, k, v)
    server = ThreadingHTTPServer(("127.0.0.1", 0), GalleryHandler)
    threading.Thread(target=server.handle_request, daemon=True).start()
    return f"http://127.0.0.1:{server.server_address[1]}"


def _get(url):
    try:
        with urllib.request.urlopen(url, timeout=5) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, b""


def _post(url, data, endpoint="/ai"):
    request = urllib.request.Request(url + endpoint, data=json.dumps(data).encode(),
                                     headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(request, timeout=5) as r:
            return r.status, json.loads(r.read())
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read())


def test_thumbs_route_serves_a_rendered_name(tmp_path):
    art = tmp_path / "a.png"
    art.write_bytes(b"\x89PNG-art")
    url = _serve_once(cache_dir=str(tmp_path / ".thumbs"),
                      thumb_src={"beef.png": str(art)}, thumb_map={})
    status, body = _get(url + "/thumbs/beef.png")
    assert status == 200 and body                       # generated into the cache on demand
    assert (tmp_path / ".thumbs" / "beef.png").exists()  # ...and kept there


def test_thumbs_route_refuses_a_name_the_page_never_rendered(tmp_path):
    url = _serve_once(cache_dir=str(tmp_path / ".thumbs"), thumb_src={}, thumb_map={})
    assert _get(url + "/thumbs/../../etc/passwd")[0] == 404


def test_search_matches_exact_prompt_text_only(tmp_path, monkeypatch):
    """Matching a Nearest's text would return images whose prompt merely
    resembles the query - the conflation the whole feature exists to prevent."""
    import subprocess
    from publicator.prompts import PromptArchive
    from publicator.webui.server import GalleryHandler
    import re, hashlib

    repo = tmp_path / "hf"; repo.mkdir()
    run = lambda *a: subprocess.run(["git", "-C", str(repo), *a], check=True,
                                    capture_output=True)
    run("init", "-q", "-b", "main")
    run("config", "user.email", "t@e"); run("config", "user.name", "t")
    (repo / "a").write_text("tentacles everywhere\n")
    run("add", "--", "a"); run("commit", "-q", "-m", "s")
    digest = hashlib.sha1(b"tentacles everywhere\n").hexdigest()

    pattern = re.compile(r"^(?P<lineage>.+)_(?P<version>[0-9a-f]{40})_"
                         r"[0-9a-f-]{36}\.[^.]+$")
    uuid = "bb6ba911-8d16-4d5a-82c4-a46b844863ed"
    picked = tmp_path / "picked"; picked.mkdir()
    hit = picked / f"a_{digest}_{uuid}.webp"
    miss = picked / f"a_{'f' * 40}_{uuid}.webp"      # never archived -> Nearest
    hit.write_bytes(b"1"); miss.write_bytes(b"2")

    # monkeypatch, not plain assignment: these are CLASS attributes and would
    # otherwise leak into every other test in this file.
    monkeypatch.setattr(GalleryHandler, "archive",
                        PromptArchive(str(repo), pattern, "sha1"))
    monkeypatch.setattr(GalleryHandler, "publicable_dirs", [str(picked)])
    monkeypatch.setattr(GalleryHandler, "json_path",
                        str(tmp_path / "publications.json"))

    monkeypatch.setattr(GalleryHandler, "data_dir", str(tmp_path))
    exact, _maybe, skipped, truncated = GalleryHandler.__new__(GalleryHandler).search("tentacles", "")
    assert exact == [str(hit)]
    assert skipped == 1, "the unarchived one is reported, not silently dropped"
    assert truncated is False


def test_prompt_search_respects_search_display_limit(workflow, monkeypatch):
    from publicator.images import Search
    from publicator.prompts import Exact, PromptVersion
    handler = workflow.gallery.GalleryHandler.__new__(workflow.gallery.GalleryHandler)
    version = PromptVersion('v', 'prompt', ('lineage',), 0)
    monkeypatch.setattr(workflow.gallery.GalleryHandler, 'archive',
                        SimpleNamespace(resolve_path=lambda path: Exact(version)))
    monkeypatch.setattr(workflow.gallery.GalleryHandler, 'publicable_dirs',
                        [str(workflow.candidates[0]).rsplit('/', 1)[0]])
    monkeypatch.setattr(workflow.gallery.GalleryHandler, 'search_state', Search(1, ('picked',)))
    monkeypatch.setattr(workflow.gallery.GalleryHandler, 'json_path', str(workflow.jp))
    monkeypatch.setattr(workflow.gallery.GalleryHandler, 'data_dir', str(workflow.jp.parent))
    exact, maybe, skipped, truncated = handler.search('', '')
    assert len(exact) == 1 and not maybe and skipped == 0 and truncated


def test_search_is_inert_when_the_feature_is_off(monkeypatch):
    from publicator.webui.server import GalleryHandler
    monkeypatch.setattr(GalleryHandler, "archive", None)
    monkeypatch.setattr(GalleryHandler, "publicable_dirs", [])
    assert GalleryHandler.__new__(GalleryHandler).search("nothing", "") == ([], [], 0, False)


def test_prompt_html_is_empty_when_the_feature_is_off(monkeypatch):
    from publicator.webui.server import GalleryHandler
    monkeypatch.setattr(GalleryHandler, "archive", None)
    assert GalleryHandler._prompt_html(GalleryHandler, ["a.webp"]) == {}


def test_skipped_excludes_a_nearest_already_shown_as_a_lineage_hint(tmp_path, monkeypatch):
    """A Nearest whose candidates match the requested lineage is shown under
    'possibly from this prompt' on a ?lineage= page — it must not ALSO
    inflate the 'skipped' banner, or the page would simultaneously claim
    those images were never archived and show them right there."""
    import subprocess
    from publicator.prompts import PromptArchive
    from publicator.webui.server import GalleryHandler
    import re

    repo = tmp_path / "hf"; repo.mkdir()
    run = lambda *a: subprocess.run(["git", "-C", str(repo), *a], check=True,
                                    capture_output=True)
    run("init", "-q", "-b", "main")
    run("config", "user.email", "t@e"); run("config", "user.name", "t")
    (repo / "a").write_text("original prompt\n")
    run("add", "--", "a"); run("commit", "-q", "-m", "s")

    pattern = re.compile(r"^(?P<lineage>.+)_(?P<version>[0-9a-f]{40})_"
                         r"[0-9a-f-]{36}\.[^.]+$")
    uuid = "bb6ba911-8d16-4d5a-82c4-a46b844863ed"
    picked = tmp_path / "picked"; picked.mkdir()
    miss = picked / f"a_{'f' * 40}_{uuid}.webp"      # never archived -> Nearest, lineage "a" known
    miss.write_bytes(b"1")

    monkeypatch.setattr(GalleryHandler, "archive",
                        PromptArchive(str(repo), pattern, "sha1"))
    monkeypatch.setattr(GalleryHandler, "publicable_dirs", [str(picked)])
    monkeypatch.setattr(GalleryHandler, "json_path",
                        str(tmp_path / "publications.json"))

    monkeypatch.setattr(GalleryHandler, "data_dir", str(tmp_path))
    exact, maybe, skipped, truncated = GalleryHandler.__new__(GalleryHandler).search("", "a")
    assert maybe == [str(miss)]
    assert skipped == 0, "already shown under 'possibly from this prompt' - not also 'skipped'"

@pytest.fixture
def workflow(tmp_path, monkeypatch):
    from publicator.webui import server as gallery
    from publicator import platforms, store
    picked = tmp_path / 'picked'; picked.mkdir()
    candidates = []
    for i in range(3):
        art = picked / f'{i}.png'; art.write_bytes(f'art-{i}'.encode())
        candidates.append(str(art))
    jp = tmp_path / 'publications.json'
    jp.write_text('[]')
    cfg = {'schedule': {'timezone': 'Europe/Paris', 'per_slot': 1},
           'tiers': ['gold'], 'galleries': ['Art'], 'publicable': ['picked']}
    submitted = []
    fail = [False]
    class Publisher:
        def configure(self, *a): pass
        def load_pending_entries(self, path):
            return [p for p in store.load_publications(path) if p['state'] == 'DESCRIBED'
                    and any(a['platformName'] == 'deviantart' and a['state'] == 'unpublished'
                            for a in p.get('apparitions', []))]
        def publish_batch(self, entries, uuids, path, **kw):
            for i, p in enumerate(entries):
                if fail[0] and i == 1:
                    return i, 1, 'stub failure'
                submitted.append((p['uuid'], p['title'], p['description']))
                store.mark_state(path, p['uuid'], 'published_or_scheduled', config=cfg)
            return len(entries), 0, None
    monkeypatch.setattr(platforms, 'PLATFORMS', {'deviantart': platforms.Platform(
        'deviantart', 'DeviantArt', Publisher(), platforms.deviantart.SKILL_MD)})
    monkeypatch.setattr(gallery, 'local_model_status', lambda m: {
        'fit': True, 'available': True, 'installed': False})
    monkeypatch.setattr(gallery, 'thumb_for_ai', lambda *a: 'thumbnail.png')
    monkeypatch.setattr(gallery, 'generate_metadata', lambda *a, **kw: ('Generated', 'Generated description'))
    args = SimpleNamespace(json=str(jp), ai_model='claude-test', openrouter_model='', ai_timeout=3)
    handlers = []
    @contextlib.contextmanager
    def start():
        handler = gallery.create_handler(str(tmp_path), candidates, args, cfg,
                                         [str(tmp_path / p) for p in cfg['publicable']])
        handlers.append(handler)
        server = ThreadingHTTPServer(('127.0.0.1', 0), handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
        try:
            yield f'http://127.0.0.1:{server.server_address[1]}'
        finally:
            server.shutdown(); thread.join(); server.server_close()
    return SimpleNamespace(start=start, jp=jp, candidates=candidates, cfg=cfg,
                           submitted=submitted, fail=fail, gallery=gallery, handlers=handlers)


def pick(url, workflow, index=0):
    status, body = _post(url, {'path': workflow.candidates[index]}, '/pick')
    assert status == 200, body
    return body['uuid']


def describe(url, uuid):
    assert _post(url, {'uuid': uuid}, '/schedule/approve')[0] == 200
    assert _post(url, {'uuid': uuid, 'title': 'Manual', 'description': 'Manual description'}, '/metadata')[0] == 200
    assert _post(url, {'uuid': uuid}, '/metadata/approve')[0] == 200


def test_http_mutations_dispatch_through_workflow_operations(workflow, monkeypatch):
    from publicator import workflow as operations
    calls = []
    for name in ('pick_publication', 'repack_publications', 'edit_publication_date',
                 'approve_publication_date', 'return_to_scheduling',
                 'save_publication_metadata', 'approve_publication_metadata',
                 'return_to_metadata', 'select_publication_platforms', 'save_platform_options'):
        operation = getattr(operations, name)
        def tracked(*args, _name=name, _operation=operation, **kwargs):
            calls.append(_name)
            return _operation(*args, **kwargs)
        monkeypatch.setattr(operations, name, tracked)
    with workflow.start() as url:
        uuid = pick(url, workflow)
        assert _post(url, {'uuid': uuid, 'scheduleProfile': 'default'}, '/schedule')[0] == 200
        assert _post(url, {'uuid': uuid}, '/schedule/approve')[0] == 200
        assert _post(url, {'uuid': uuid, 'title': 'Manual', 'description': 'Description'}, '/metadata')[0] == 200
        assert _post(url, {'uuid': uuid}, '/metadata/approve')[0] == 200
        assert _post(url, {'uuid': uuid, 'platform': 'deviantart', 'selected': True}, '/selection')[0] == 200
        assert _post(url, {'selected': True}, '/selection/all')[0] == 200
        assert _post(url, {'uuid': uuid, 'platform': 'deviantart', 'price': 2}, '/options')[0] == 200
        assert _post(url, {'uuid': uuid}, '/metadata/return')[0] == 200
        assert _post(url, {'uuid': uuid}, '/schedule/return')[0] == 200
    assert calls == ['pick_publication', 'repack_publications', 'edit_publication_date',
                     'approve_publication_date', 'save_publication_metadata',
                     'approve_publication_metadata', 'select_publication_platforms',
                     'select_publication_platforms', 'save_platform_options',
                     'return_to_metadata', 'return_to_scheduling']


def test_search_reload_is_allowlisted_and_replaces_candidate_authorization(workflow):
    from pathlib import Path
    from publicator.images import Search
    extra = Path(workflow.candidates[0]).parent.parent / 'other'
    extra.mkdir()
    dropped = extra / 'dropped.png'
    dropped.write_bytes(b'dropped batch')
    workflow.cfg['publicable'].append('other')
    with workflow.start() as url:
        assert workflow.handlers[-1].search_state == Search(25, ('picked', 'other'))
        response = _post(url, {'limit': 25, 'paths': ['other']}, '/search')
        assert response == (200, {'limit': 25, 'paths': ['other']})
        assert b'data-path="' + str(dropped).encode() + b'"' in _get(url)[1]
        assert _post(url, {'limit': 0, 'paths': ['picked']}, '/search')[0] == 200
        assert workflow.handlers[-1].candidate_paths == []
        assert _post(url, {'path': str(dropped)}, '/pick')[0] == 400
        assert _get(url)[0] == 200
        assert _post(url, {'limit': 25, 'paths': ['picked']}, '/search')[0] == 200
        assert _post(url, {'limit': 25, 'paths': ['unknown']}, '/search')[0] == 400
        for body in (
                {'limit': 25, 'paths': ['picked', 'picked']},
                {'limit': 24, 'paths': ['picked']},
                {'limit': True, 'paths': ['picked']},
                {'limit': 25, 'paths': '../picked'},
                {'limit': 25, 'paths': [1]},
                {'limit': 25, 'paths': ['picked'], 'extra': True}):
            assert _post(url, body, '/search')[0] == 400


def test_page_render_cannot_restore_candidates_after_search_reload(workflow, monkeypatch):
    from threading import Event, Thread

    with workflow.start() as url:
        handler = workflow.handlers[-1]
        page_handler = handler.__new__(handler)
        candidates_snapshotted = Event()
        resume_render = Event()
        original_candidates = page_handler._candidates

        def blocked_candidates():
            snapshot = original_candidates()
            candidates_snapshotted.set()
            assert resume_render.wait(timeout=5)
            return snapshot

        monkeypatch.setattr(page_handler, "_candidates", blocked_candidates)
        render_errors = []

        def render_page():
            try:
                page_handler._build_page()
            except Exception as exc:
                render_errors.append(exc)

        render = Thread(target=render_page)
        render.start()
        assert candidates_snapshotted.wait(timeout=5)
        assert _post(url, {"limit": 0, "paths": ["picked"]}, "/search")[0] == 200
        resume_render.set()
        render.join(timeout=5)
        assert not render.is_alive()
        assert not render_errors

        assert _post(url, {"path": workflow.candidates[0]}, "/pick")[0] == 400


def test_pick_persists_without_metadata_and_reopens_in_schedule(workflow):
    with workflow.start() as url:
        uuid = pick(url, workflow)
        assert pick(url, workflow) == uuid
        row, = json.loads(workflow.jp.read_text())
        assert row['state'] == 'PICKED' and not row.get('apparitions')
        assert row['title'] == row['description'] == ''
    with workflow.start() as url:
        status, page = _get(url)
        assert status == 200 and uuid.encode() in page
        assert b'id="schedule-tab"' in page
        assert _post(url, {'uuid': uuid}, '/publish')[0] == 400


def test_schedule_repacks_pins_and_approval_freezes(workflow):
    with workflow.start() as url:
        one, two = pick(url, workflow, 0), pick(url, workflow, 1)
        rows = json.loads(workflow.jp.read_text())
        first = rows[0]['apparitionTimestampIfDifferentThanSubmission']
        assert rows[1]['apparitionTimestampIfDifferentThanSubmission'] > first
        assert _post(url, {'uuid': two, 'scheduleTs': first}, '/schedule')[0] == 200
        rows = json.loads(workflow.jp.read_text())
        assert rows[0]['apparitionTimestampIfDifferentThanSubmission'] > first
        assert rows[1]['scheduleIsManual'] is True
        assert _post(url, {'uuid': two}, '/schedule/approve')[0] == 200
        frozen = workflow.jp.read_text()
        assert _post(url, {'uuid': two, 'schedule': '2027-01-01T20:00'}, '/schedule')[0] == 400
        assert workflow.jp.read_text() == frozen
        assert _post(url, {'uuid': one, 'scheduleIsManual': False}, '/schedule')[0] == 200
        assert json.loads(workflow.jp.read_text())[1]['apparitionTimestampIfDifferentThanSubmission'] == first
        assert b'Over capacity' not in _get(url)[1]  # automatic row moved out


def test_ai_batch_is_last_first_exact_marker_only_and_continues_after_failure(workflow, monkeypatch):
    from publicator.llm_meta import TODO_GENERATE, VISION_MODELS
    generated = []
    def provider(image, model, **kwargs):
        generated.append(image)
        if image == 'thumbnail.png' and len(generated) == 2:
            raise RuntimeError('provider failed')
        return 'Generated', 'Description'
    monkeypatch.setattr(workflow.gallery, 'generate_metadata', provider)
    with workflow.start() as url:
        uuids = [pick(url, workflow, i) for i in range(3)]
        for uuid in uuids:
            assert _post(url, {'uuid': uuid}, '/schedule/approve')[0] == 200
        rows = json.loads(workflow.jp.read_text())
        rows[0]['title'] = 'Manual title'
        rows[0]['description'] = TODO_GENERATE
        rows[1]['title'] = TODO_GENERATE
        rows[1]['description'] = 'Manual description'
        rows[2]['title'] = TODO_GENERATE + ' '  # not an exact marker
        workflow.jp.write_text(json.dumps(rows))
        html = _get(url)[1]
        assert b'Describe all marked' in html
        status, body = _post(url, {'source': 'openrouter', 'model': VISION_MODELS[0]}, '/ai/batch')
        assert status == 200 and body['succeeded'] == 1 and body['failed'] == 1
        saved = json.loads(workflow.jp.read_text())
        by_id = {p['uuid']: p for p in saved}
        assert by_id[uuids[0]]['title'] == 'Manual title'
        assert by_id[uuids[0]]['description'] == TODO_GENERATE
        assert by_id[uuids[1]]['title'] == 'Generated'
        assert by_id[uuids[1]]['description'] == 'Manual description'
        assert by_id[uuids[2]]['title'] == TODO_GENERATE + ' '
        assert [r['uuid'] for r in body['results']] == [uuids[1], uuids[0]]


def test_ai_batch_keeps_persisted_order_for_non_openrouter_provider(workflow, monkeypatch):
    calls = []
    monkeypatch.setattr(workflow.gallery, 'thumb_for_ai', lambda path, *a: path.rsplit('/', 1)[-1])
    monkeypatch.setattr(workflow.gallery, 'generate_metadata',
                        lambda image, *a, **kw: calls.append(image) or ('T', 'D'))
    with workflow.start() as url:
        uuids = [pick(url, workflow, i) for i in range(2)]
        for uuid in uuids:
            assert _post(url, {'uuid': uuid}, '/schedule/approve')[0] == 200
        from publicator.llm_meta import TODO_GENERATE
        rows = json.loads(workflow.jp.read_text())
        for row in rows:
            row['title'] = row['description'] = TODO_GENERATE
        workflow.jp.write_text(json.dumps(rows))
        status, body = _post(url, {'source': 'claude'}, '/ai/batch')
        assert status == 200 and body['succeeded'] == 2
        assert calls == ['0.png', '1.png']


def test_ai_saves_complete_pair_and_retries_failure_markers(workflow, monkeypatch):
    from publicator.llm_meta import TODO_GENERATE
    generated_pairs = []
    save_generated = workflow.gallery.workflow.save_generated_metadata
    def track_generated(*args, **kwargs):
        generated_pairs.append(args[2:4])
        return save_generated(*args, **kwargs)
    monkeypatch.setattr(workflow.gallery.workflow, 'save_generated_metadata', track_generated)
    with workflow.start() as url:
        uuid = pick(url, workflow)
        assert _post(url, {'uuid': uuid}, '/schedule/approve')[0] == 200
        assert _post(url, {'uuid': uuid, 'title': 'Keep me', 'description': ''}, '/metadata')[0] == 200
        def fail(*a, **kw): raise RuntimeError('provider failed <script>')
        monkeypatch.setattr(workflow.gallery, 'generate_metadata', fail)
        status, body = _post(url, {'uuid': uuid, 'source': 'claude'}, '/ai')
        assert status == 400 and 'provider failed' in body['error'] and body['saved'] is True
        row, = json.loads(workflow.jp.read_text())
        assert row['title'] == row['description'] == TODO_GENERATE and row['state'] == 'SCHEDULED'
        assert generated_pairs == [(TODO_GENERATE, TODO_GENERATE)]
        assert _post(url, {'uuid': uuid}, '/metadata/approve')[0] == 400
    with workflow.start() as url:
        assert b'AI metadata pending' in _get(url)[1]
        monkeypatch.setattr(workflow.gallery, 'generate_metadata', lambda *a, **kw: ('New', 'Done'))
        assert _post(url, {'uuid': uuid, 'source': 'claude'}, '/ai')[0] == 200
        row, = json.loads(workflow.jp.read_text())
        assert row['title'] == 'New' and row['description'] == 'Done' and row['state'] == 'DESCRIBED'
        assert generated_pairs[-1] == ('New', 'Done')
        assert _post(url, {'uuid': uuid}, '/metadata/approve')[0] == 400


def test_ai_ineligible_phase_is_rejected_by_workflow_before_provider(workflow, monkeypatch):
    from publicator import workflow as operations
    provider_calls = []
    guard_calls = []
    guard = operations.require_metadata_generation
    def track_guard(publication):
        guard_calls.append(publication['uuid'])
        return guard(publication)
    monkeypatch.setattr(operations, 'require_metadata_generation', track_guard)
    monkeypatch.setattr(workflow.gallery, 'generate_metadata', lambda *a, **kw: provider_calls.append(a))
    with workflow.start() as url:
        uuid = pick(url, workflow)
        before = workflow.jp.read_text()
        status, body = _post(url, {'uuid': uuid, 'source': 'claude'}, '/ai')
        assert status == 400 and body['error'] == 'AI generation requires SCHEDULED'
        assert guard_calls == [uuid]
        assert not provider_calls
        assert workflow.jp.read_text() == before


def test_ai_validation_does_not_mutate_and_uses_canonical_models(workflow, monkeypatch):
    from publicator.llm_meta import VISION_MODELS
    calls = []
    monkeypatch.setattr(workflow.gallery, 'generate_metadata', lambda *a, **kw: calls.append((a, kw)) or ('t', 'd'))
    with workflow.start() as url:
        uuid = pick(url, workflow)
        base = {'uuid': uuid, 'source': 'claude'}
        before = workflow.jp.read_text()
        assert _post(url, base, '/ai')[0] == 400  # wrong phase
        assert workflow.jp.read_text() == before
        assert _post(url, {'uuid': uuid}, '/schedule/approve')[0] == 200
        before = workflow.jp.read_text()
        for extra in ({'source': 'unknown'}, {'fields': ['title']},
                      {'path': '/etc/passwd'}, {'model': {}}, {'uuid': 'missing'},
                      {'source': 'openrouter', 'model': {**VISION_MODELS[0], 'llmfit_name': 'forged'}}):
            assert _post(url, {**base, **extra}, '/ai')[0] in (400, 404)
            assert workflow.jp.read_text() == before
        assert not calls
        model = VISION_MODELS[0]
        assert _post(url, {**base, 'source': 'local', 'model': model}, '/ai')[0] == 200
        assert calls[-1][0] == ('thumbnail.png', model)
        assert calls[-1][1]['install_missing'] is True


def test_selections_all_options_success_and_retry_restore(workflow):
    with workflow.start() as url:
        one, two = pick(url, workflow, 0), pick(url, workflow, 1)
        describe(url, one); describe(url, two)
        assert _post(url, {'uuid': one, 'platform': 'deviantart', 'selected': True}, '/selection')[0] == 200
        assert _post(url, {'uuid': one, 'platform': 'deviantart', 'price': 3, 'tiers': ['gold'], 'galleries': ['Art']}, '/options')[0] == 200
        assert _post(url, {'selected': True}, '/selection/all')[0] == 200
        assert len(json.loads(workflow.jp.read_text())[1]['apparitions']) == 1
        workflow.fail[0] = True
        status, result = _post(url, {}, '/publish')
        assert status == 200 and result['published'] == 1 and result['failed'] == 1
        assert result['remaining'] == [{'uuid': two, 'platform': 'deviantart'}]
        rows = json.loads(workflow.jp.read_text())
        assert rows[0]['state'] == 'APPEARED' and rows[1]['state'] == 'DESCRIBED'
        assert rows[0]['apparitions'][0]['priceIfNotFree'] == 3
        assert _post(url, {'uuid': one, 'platform': 'deviantart', 'selected': False}, '/selection')[0] == 400
        assert _get(url)[0] == 200  # server remains alive
    with workflow.start() as url:
        page = _get(url)[1]
        assert b'Published / scheduled' in page and b'disabled' in page
        workflow.fail[0] = False
        assert _post(url, {}, '/publish')[1]['published'] == 1
        assert [u for u, *_ in workflow.submitted] == [one, two]
        assert _post(url, {}, '/publish')[1]['published'] == 0


def test_http_boundary_rejects_paths_obsolete_fields_and_unknown_platforms(workflow):
    with workflow.start() as url:
        before = workflow.jp.read_text()
        for endpoint, body in [('/pick', {'path': '/etc/passwd'}), ('/delete', {'path': '/etc/passwd'}),
                               ('/stage', {'entries': []}), ('/update', {'uuid': 'x'})]:
            assert _post(url, body, endpoint)[0] in (400, 404)
            assert workflow.jp.read_text() == before
        uuid = pick(url, workflow)
        assert _post(url, {'path': workflow.candidates[0]}, '/delete')[0] == 400
        from pathlib import Path
        artwork = Path(workflow.candidates[0])
        artwork.write_bytes(b'edited after picking')
        page = _get(url)[1]
        assert _post(url, {'path': str(artwork)}, '/delete')[0] == 400
        assert artwork.read_bytes() == b'edited after picking'
        assert f'data-path="{artwork}"'.encode() not in page
        describe(url, uuid)
        before = workflow.jp.read_text()
        for endpoint, body in [('/selection', {'uuid': uuid, 'platform': 'pixiv', 'selected': True}),
                               ('/selection', {'uuid': uuid, 'platform': 'deviantart', 'selected': 'yes'}),
                               ('/metadata', {'uuid': uuid, 'urlElsePublicationName': 'bad'}),
                               ('/publish', {'entries': [{'title': 'forged'}]}),
                               ('/options', {'uuid': uuid, 'platform': 'deviantart', 'url': 'http://x'})]:
            assert _post(url, body, endpoint)[0] == 400
            assert workflow.jp.read_text() == before
        assert _post(url, {'uuid': uuid}, '/metadata/return')[0] == 200
        assert _post(url, {'uuid': uuid, 'title': '[[PUBLICATOR:TODO_GENERATE]] embedded'}, '/metadata')[0] == 200
        assert _post(url, {'uuid': uuid}, '/metadata/approve')[0] == 400
        assert _post(url, {'uuid': uuid}, '/publish')[0] == 400
        assert not workflow.submitted


@pytest.mark.parametrize('content_type, body, origin, host', [
    ('application/x-www-form-urlencoded', b'', None, None),
    ('application/x-www-form-urlencoded', b'', 'https://untrusted.example', None),
    ('text/plain', b'', None, None),
    ('text/plain', b'{}', None, None),
    (None, b'', None, None),
    ('application/json', b'{}', 'https://untrusted.example', None),
    ('application/json', b'{}', 'null', None),
    ('application/json', b'{}', 'http://127.0.0.1:1', None),
    ('application/json', b'{}', 'http://untrusted.example', 'untrusted.example'),
])
def test_mutations_reject_non_json_and_foreign_origins_before_submission(
        workflow, content_type, body, origin, host):
    with workflow.start() as url:
        uuid = pick(url, workflow); describe(url, uuid)
        assert _post(url, {'uuid': uuid, 'platform': 'deviantart', 'selected': True}, '/selection')[0] == 200
        before = workflow.jp.read_text()
        headers = {}
        if content_type is not None:
            headers['Content-Type'] = content_type
        if origin is not None:
            headers['Origin'] = origin
        if host is not None:
            headers['Host'] = host
        # A foreign form needs no UUID or knowledge of the saved selections.
        # http.client leaves Content-Type absent when requested (urllib adds a form default).
        with contextlib.closing(http.client.HTTPConnection(urllib.parse.urlsplit(url).netloc, timeout=5)) as connection:
            connection.request('POST', '/publish', body=body, headers=headers)
            rejected = connection.getresponse()
            assert rejected.status == (415 if content_type != 'application/json' else 403)
            assert 'error' in json.loads(rejected.read())
            assert not workflow.submitted
            assert workflow.jp.read_text() == before
            # The same boundary protects deletion and other mutations, not just publish.
            connection.request('POST', '/delete',
                body=json.dumps({'path': workflow.candidates[1]}).encode(), headers=headers)
            rejected = connection.getresponse()
            assert rejected.status == (415 if content_type != 'application/json' else 403)
            rejected.read()
        from pathlib import Path
        assert Path(workflow.candidates[1]).exists()
        assert workflow.jp.read_text() == before
        # A real browser's same-origin JSON request (including a charset) still works.
        request = urllib.request.Request(url + '/publish', data=b'{}',
            headers={'Content-Type': 'application/json; charset=utf-8', 'Origin': url})
        with urllib.request.urlopen(request, timeout=5) as response:
            assert response.status == 200 and json.loads(response.read())['published'] == 1
        assert [u for u, *_ in workflow.submitted] == [uuid]


@pytest.mark.parametrize('names', [('pixiv',), ('deviantart', 'pixiv')])
def test_historical_calendar_reopens_with_every_platform_and_shared_prompt(workflow, monkeypatch, names):
    import re
    from datetime import datetime
    from zoneinfo import ZoneInfo
    from publicator.config import validate_publications
    from publicator.prompts import Exact, PromptVersion
    with workflow.start() as url:
        uuid = pick(url, workflow); describe(url, uuid)
    rows = json.loads(workflow.jp.read_text())
    row, = rows
    row['state'] = 'APPEARED'
    row['title'] = 'Shared historical title'
    row['apparitionTimestampIfDifferentThanSubmission'] = int(datetime(
        2026, 10, 11, 0, 30, tzinfo=ZoneInfo('Europe/Paris')).timestamp())
    row['apparitions'] = [{'platformName': name, 'state': 'published_or_scheduled',
                          'url': f'https://{name}.example/artwork/123'} for name in names]
    validate_publications(rows, workflow.cfg)
    workflow.jp.write_text(json.dumps(rows))
    before = workflow.jp.read_text()
    workflow.candidates.clear()  # Reopen a history-only server with no new candidates.
    version = PromptVersion('historical-version', 'Shared historical prompt', ('lineage',), 0)
    monkeypatch.setattr(workflow.gallery, 'prompt_archive_from_config', lambda *a: SimpleNamespace(
        resolve_path=lambda path: Exact(version)))
    with workflow.start() as url:
        status, page = _get(url)
        assert status == 200
        text = page.decode()
        calendar = text.split('id="calendar-tab"', 1)[1].split('</section></main>', 1)[0]
        day = re.search(r'<td[^>]*data-date="2026-10-11"[^>]*>(.*?)</td>', calendar, re.S).group(1)
        assert day.count('Shared historical title') >= len(names)
        assert day.count('Shared historical prompt') == len(names)
        ids = re.findall(r'id="(evp[^"]+)" popover', day)
        assert len(ids) == len(set(ids)) == len(names)
        for name in names:
            assert f'href="https://{name}.example/artwork/123"' in day
            label = {'deviantart': 'DeviantArt', 'pixiv': 'Pixiv'}[name]
            assert f'open on {label} ↗' in day
        thumbs = re.findall(r'src="(/thumbs/[^"]+)"', day)
        assert len(thumbs) == len(names)
        assert all(_get(url + thumb)[0] == 200 for thumb in thumbs)
        assert _get(url + '/thumbs/not-rendered.png')[0] == 404
        assert _post(url, {}, '/publish')[1]['published'] == 0
        assert not workflow.submitted and workflow.jp.read_text() == before


@pytest.mark.parametrize('by_uuid', [False, True])
def test_historical_only_pending_publish_is_an_unchanged_noop(workflow, monkeypatch, by_uuid):
    from publicator.config import validate_publications
    calls = []
    dispatch = workflow.gallery.publish_selected
    def publish(*args, **kwargs):
        calls.append(kwargs)
        return dispatch(*args, **kwargs)
    monkeypatch.setattr(workflow.gallery, 'publish_selected', publish)
    with workflow.start() as url:
        uuid = pick(url, workflow); describe(url, uuid)
        rows = json.loads(workflow.jp.read_text())
        rows[0]['apparitions'] = [{'platformName': 'pixiv', 'state': 'unpublished'}]
        # Historical-only content is not executable preflight's concern.
        rows[0]['title'] = '[[PUBLICATOR:TODO_GENERATE]]'
        validate_publications(rows, workflow.cfg)
        workflow.jp.write_text(json.dumps(rows))
        before = workflow.jp.read_text()
        status, result = _post(url, {'uuid': uuid} if by_uuid else {}, '/publish')
        assert status == 200, result
        assert result == {'published': 0, 'failed': 0, 'unattempted': 0, 'remaining': [], 'error': None}
        assert not workflow.submitted and workflow.jp.read_text() == before
        assert calls == [{'uuids': [], 'platform_names': ['deviantart'], 'config': workflow.cfg}]
        assert _post(url, {'uuid': uuid, 'platform': 'pixiv', 'selected': True}, '/selection')[0] == 400
        assert workflow.jp.read_text() == before


def test_mixed_historical_pending_publish_counts_and_retry_only_registered_cells(workflow, monkeypatch):
    from publicator.config import validate_publications
    from publicator import store
    calls = []
    dispatch = workflow.gallery.publish_selected
    def publish(*args, **kwargs):
        calls.append(kwargs)
        return dispatch(*args, **kwargs)
    monkeypatch.setattr(workflow.gallery, 'publish_selected', publish)
    with workflow.start() as url:
        uuids = [pick(url, workflow, i) for i in range(3)]
        for uuid in uuids: describe(url, uuid)
        assert _post(url, {'selected': True}, '/selection/all')[0] == 200
        rows = json.loads(workflow.jp.read_text())
        historical = {'platformName': 'pixiv', 'state': 'unpublished', 'url': 'https://pixiv.example/artwork/123'}
        for row in rows:
            row['apparitions'].append(dict(historical))
        # A fourth historical-only publication must not block executable rows.
        history_only = {**rows[0], 'uuid': '00000000-0000-4000-8000-000000000099',
                        'title': '[[PUBLICATOR:TODO_GENERATE]]', 'apparitions': [dict(historical)]}
        rows.append(history_only)
        validate_publications(rows, workflow.cfg)
        workflow.jp.write_text(json.dumps(rows))
        workflow.fail[0] = True
        status, result = _post(url, {}, '/publish')
        assert status == 200, result
        assert result['published'] == result['failed'] == result['unattempted'] == 1
        assert result['remaining'] == [{'uuid': u, 'platform': 'deviantart'} for u in uuids[1:]]
        assert calls[-1] == {'uuids': uuids, 'platform_names': ['deviantart'], 'config': workflow.cfg}
        rows = store.load_publications(str(workflow.jp))
        assert all(row['state'] == 'DESCRIBED' for row in rows)
        assert all(row['apparitions'][-1] == historical for row in rows)
        assert rows[0]['apparitions'][0]['state'] == 'published_or_scheduled'
        workflow.fail[0] = False
        status, result = _post(url, {}, '/publish')
        assert status == 200 and result['published'] == 2
        assert result['failed'] == result['unattempted'] == 0 and result['remaining'] == []
        assert calls[-1]['uuids'] == uuids[1:]
        rows = store.load_publications(str(workflow.jp))
        assert all(row['state'] == 'DESCRIBED' for row in rows)  # Historical delivery still pending.
        assert all(row['apparitions'][-1] == historical for row in rows)
        assert rows[-1] == history_only
        assert [u for u, *_ in workflow.submitted] == uuids
        assert _post(url, {}, '/publish')[1]['published'] == 0
        assert calls[-1]['uuids'] == []
        assert [u for u, *_ in workflow.submitted] == uuids


def test_invalid_json_is_not_an_empty_queue(workflow):
    workflow.jp.write_text('{broken')
    with workflow.start() as url:
        assert _get(url)[0] == 500
        assert _post(url, {'path': workflow.candidates[0]}, '/pick')[0] == 400
        assert workflow.jp.read_text() == '{broken'


def test_ai_does_not_hold_mutation_lock_and_preserves_unrequested_edits(workflow, monkeypatch):
    from publicator import store
    with workflow.start() as url:
        uuid = pick(url, workflow)
        assert _post(url, {'uuid': uuid}, '/schedule/approve')[0] == 200
        def generate(*a, **kw):
            # A different HTTP thread must be able to complete a store mutation.
            assert _post(url, {'uuid': uuid, 'description': 'Concurrent draft'}, '/metadata')[0] == 200
            return 'AI title', 'Ignore me'
        monkeypatch.setattr(workflow.gallery, 'generate_metadata', generate)
        assert _post(url, {'uuid': uuid, 'source': 'claude'}, '/ai')[0] == 200
        row, = store.load_publications(str(workflow.jp))
        assert row['title'] == 'AI title' and row['description'] == 'Ignore me'


def test_busy_publish_rejects_duplicate_batches(workflow, monkeypatch):
    entered, release = threading.Event(), threading.Event()
    def publishing(*a, **kw):
        entered.set(); assert release.wait(4)
        return 0, 1, 'stub failure'
    monkeypatch.setattr(workflow.gallery, 'publish_selected', publishing)
    with workflow.start() as url:
        uuid = pick(url, workflow); describe(url, uuid)
        assert _post(url, {'uuid': uuid, 'platform': 'deviantart', 'selected': True}, '/selection')[0] == 200
        results = []
        t = threading.Thread(target=lambda: results.append(_post(url, {}, '/publish'))); t.start()
        assert entered.wait(3)
        try:
            assert _post(url, {}, '/publish')[0] == 409
            assert _get(url)[0] == 200
        finally:
            release.set(); t.join()
        assert results[0][0] == 200


@pytest.mark.parametrize('history_platform', ['deviantart', 'pixiv'])
def test_ui_launches_history_only_and_draft_only(tmp_path, monkeypatch, history_platform):
    from publicator.apps import ui
    (tmp_path / 'publicator.toml').write_text('publicable = ["picked", "other"]\n')
    from publicator import store
    art = tmp_path / 'a.png'; art.write_bytes(b'art')
    jp = tmp_path / 'publications.json'
    uuid = store.pick_publication(str(art), str(jp), {})
    calls = []
    searches = []
    monkeypatch.setattr(ui, 'serve', lambda *args: calls.append(args))
    monkeypatch.setattr(ui, 'find_candidates', lambda search, path: searches.append(search) or [])
    monkeypatch.setattr('sys.argv', ['ui', '--data-dir', str(tmp_path)])
    assert ui.main() == 0 and len(calls) == 1
    from publicator.images import Search
    assert searches == [Search(25, (str(tmp_path / 'picked'), str(tmp_path / 'other')))]
    monkeypatch.setattr('sys.argv', ['ui', '-n', '10', '--data-dir', str(tmp_path)])
    with pytest.raises(SystemExit):
        ui.main()
    store.repack_publications(str(jp), {})
    store.approve_publication_date(str(jp), uuid, {})
    store.save_publication_metadata(str(jp), uuid, title='t', description='d', config={})
    store.approve_publication_metadata(str(jp), uuid, {})
    store.select_publication_platforms(str(jp), selected=True, registered_platforms=['deviantart'],
                                      uuid_=uuid, platform='deviantart', config={})
    store.mark_state(str(jp), uuid, 'published_or_scheduled', config={})
    rows = store.load_publications(str(jp))
    rows[0]['apparitions'][0]['platformName'] = history_platform
    from publicator.config import validate_publications
    validate_publications(rows, {})
    jp.write_text(json.dumps(rows))
    monkeypatch.setattr('sys.argv', ['ui', '--data-dir', str(tmp_path)])
    assert ui.main() == 0 and len(calls) == 2
    assert calls[-1][1] == []  # No candidate images; even unregistered history launches.


def test_accepted_generation_blocks_approval_and_duplicate_ai_until_saved(workflow, monkeypatch):
    from publicator.llm_meta import TODO_GENERATE
    with workflow.start() as url:
        uuid = pick(url, workflow)
        assert _post(url, {'uuid': uuid}, '/schedule/approve')[0] == 200
        assert _post(url, {'uuid': uuid, 'title': 'Manual', 'description': 'Manual'}, '/metadata')[0] == 200
        request = {'uuid': uuid, 'source': 'claude'}
        def generate(*a, **kw):
            assert _post(url, {'uuid': uuid}, '/metadata/approve')[0] == 409
            assert _post(url, {'uuid': uuid}, '/schedule/return')[0] == 409
            assert _post(url, request, '/ai')[0] == 409
            raise RuntimeError('accepted generation failed')
        monkeypatch.setattr(workflow.gallery, 'generate_metadata', generate)
        status, result = _post(url, request, '/ai')
        assert status == 400 and result['saved'] is True
        row, = json.loads(workflow.jp.read_text())
        assert row['state'] == 'SCHEDULED' and row['title'] == TODO_GENERATE
        assert _post(url, {'uuid': uuid}, '/metadata/approve')[0] == 400


def test_new_registered_target_on_appeared_preserves_success_and_locks_metadata(workflow, monkeypatch):
    from publicator import platforms
    with workflow.start() as url:
        uuid = pick(url, workflow); describe(url, uuid)
        assert _post(url, {'uuid': uuid, 'platform': 'deviantart', 'selected': True}, '/selection')[0] == 200
        assert _post(url, {}, '/publish')[1]['published'] == 1
        monkeypatch.setitem(platforms.PLATFORMS, 'stub', platforms.Platform('stub', 'Stub', object(), platforms.deviantart.SKILL_MD))
        assert _post(url, {'selected': True}, '/selection/all')[0] == 200
        row, = json.loads(workflow.jp.read_text())
        assert row['state'] == 'DESCRIBED'
        assert [a['state'] for a in row['apparitions']] == ['published_or_scheduled', 'unpublished']
        for endpoint in ('/schedule/return', '/metadata/return'):
            assert _post(url, {'uuid': uuid}, endpoint)[0] == 400
        assert _post(url, {'uuid': uuid, 'platform': 'deviantart', 'price': 5}, '/options')[0] == 400
        assert _post(url, {'selected': False}, '/selection/all')[0] == 200
        row, = json.loads(workflow.jp.read_text())
        assert [a['platformName'] for a in row['apparitions']] == ['deviantart']
        assert row['state'] == 'APPEARED'  # withdrawing the only pending target restores completion


@pytest.mark.parametrize('exception', [RuntimeError('invalid output'), OSError('command start failed'), ValueError('unexpected bug')])
def test_all_accepted_generation_failures_save_both_markers(workflow, monkeypatch, exception):
    from publicator.llm_meta import TODO_GENERATE
    def generate(*a, **kw): raise exception
    monkeypatch.setattr(workflow.gallery, 'generate_metadata', generate)
    with workflow.start() as url:
        uuid = pick(url, workflow)
        assert _post(url, {'uuid': uuid}, '/schedule/approve')[0] == 200
        status, result = _post(url, {'uuid': uuid, 'source': 'claude'}, '/ai')
        assert status == (500 if isinstance(exception, ValueError) else 400)
        assert result['saved'] is True and result['error'] == str(exception)
        row, = json.loads(workflow.jp.read_text())
        assert row['title'] == row['description'] == TODO_GENERATE


def test_overcapacity_and_exhausted_horizon_remain_visible(workflow, monkeypatch):
    from publicator import scheduling
    monkeypatch.setattr(scheduling, 'SLOT_HORIZON_WEEKS', 1)
    with workflow.start() as url:
        one, two = pick(url, workflow), pick(url, workflow, 1)
        assert b'No slot available' in _get(url)[1]
        assert _post(url, {'uuid': two}, '/schedule/approve')[0] == 400
        slot = json.loads(workflow.jp.read_text())[0]['apparitionTimestampIfDifferentThanSubmission']
        assert _post(url, {'uuid': one, 'scheduleTs': slot}, '/schedule')[0] == 200
        assert _post(url, {'uuid': two, 'scheduleTs': slot}, '/schedule')[0] == 200
        assert b'Over capacity' in _get(url)[1]


def test_da_title_limit_and_options_validation_do_not_write(workflow):
    with workflow.start() as url:
        uuid = pick(url, workflow)
        describe(url, uuid)
        assert _post(url, {'uuid': uuid}, '/metadata/return')[0] == 200
        assert _post(url, {'uuid': uuid, 'title': 'x' * 51}, '/metadata')[0] == 200
        assert _post(url, {'uuid': uuid}, '/metadata/approve')[0] == 200  # shared title has no DA-only limit
        before = workflow.jp.read_text()
        assert _post(url, {'uuid': uuid, 'platform': 'deviantart', 'selected': True}, '/selection')[0] == 400
        assert workflow.jp.read_text() == before
        assert _post(url, {'uuid': uuid}, '/metadata/return')[0] == 200
        assert _post(url, {'uuid': uuid, 'title': 'Valid'}, '/metadata')[0] == 200
        assert _post(url, {'uuid': uuid}, '/metadata/approve')[0] == 200
        assert _post(url, {'uuid': uuid, 'platform': 'deviantart', 'selected': True}, '/selection')[0] == 200
        before = workflow.jp.read_text()
        for extra in ({'tiers': ['forged']}, {'galleries': ['forged']}, {'price': -1}, {'price': float('inf')}):
            assert _post(url, {'uuid': uuid, 'platform': 'deviantart', **extra}, '/options')[0] == 400
            assert workflow.jp.read_text() == before


def test_publish_result_distinguishes_failed_from_unattempted(workflow):
    with workflow.start() as url:
        uuids = [pick(url, workflow, i) for i in range(3)]
        for uuid in uuids: describe(url, uuid)
        assert _post(url, {'selected': True}, '/selection/all')[0] == 200
        workflow.fail[0] = True
        result = _post(url, {}, '/publish')[1]
        assert result['published'] == result['failed'] == result['unattempted'] == 1
        assert result['remaining'] == [{'uuid': u, 'platform': 'deviantart'} for u in uuids[1:]]


@pytest.mark.parametrize('nearest', [False, True])
def test_search_excludes_referenced_artwork_with_changed_bytes(workflow, monkeypatch, nearest):
    from pathlib import Path
    from publicator.prompts import Exact, Nearest, PromptVersion, Lineage
    version = PromptVersion('version', 'a prompt', ('lineage',), 0)
    match = Nearest((Lineage('lineage', (version,)),)) if nearest else Exact(version)
    monkeypatch.setattr(workflow.gallery, 'prompt_archive_from_config', lambda *a: SimpleNamespace(
        resolve_path=lambda path: match))
    with workflow.start() as url:
        pick(url, workflow)
        artwork = Path(workflow.candidates[0])
        artwork.write_bytes(b'edited after picking')
        status, page = _get(url + '/?lineage=lineage')
        assert status == 200
        assert _post(url, {'path': str(artwork)}, '/delete')[0] == 400
        assert artwork.read_bytes() == b'edited after picking'
        assert f'data-path="{artwork}"'.encode() not in page
        assert f'data-path="{workflow.candidates[1]}"'.encode() in page


def test_ai_generation_disables_metadata_inputs_and_reenables_after_failure(workflow, monkeypatch):
    from playwright.sync_api import sync_playwright

    entered, release = threading.Event(), threading.Event()
    def generate(*a, **kw):
        entered.set()
        assert release.wait(timeout=5)
        raise RuntimeError('stub provider failure')
    monkeypatch.setattr(workflow.gallery, 'generate_metadata', generate)
    with workflow.start() as url:
        uuid = pick(url, workflow)
        assert _post(url, {'uuid': uuid}, '/schedule/approve')[0] == 200
        with sync_playwright() as pw:
            browser = pw.firefox.launch(headless=True)
            try:
                page = browser.new_page()
                page.goto(url + '/#describe')
                page.locator('#ai-model').select_option(label='Claude (subscription) — claude-test')
                page.locator('[data-action=ai]').evaluate('e => e.click()')
                assert entered.wait(timeout=5)
                title = page.locator(f'#publication-{uuid} [data-field=title]')
                description = page.locator(f'#publication-{uuid} [data-field=description]')
                assert title.is_disabled()
                assert description.is_disabled()
                release.set()
                page.wait_for_function("!document.querySelector('[data-action=ai]').disabled")
                assert title.is_enabled()
                assert description.is_enabled()
            finally:
                release.set()
                browser.close()


def test_ai_button_requests_both_fields_and_refreshes_to_publish(workflow, monkeypatch):
    from playwright.sync_api import sync_playwright

    monkeypatch.setattr(workflow.gallery, 'generate_metadata',
                        lambda *a, **kw: ('Generated title', 'Generated description'))
    with workflow.start() as url:
        uuid = pick(url, workflow)
        assert _post(url, {'uuid': uuid}, '/schedule/approve')[0] == 200
        with sync_playwright() as pw:
            browser = pw.firefox.launch(headless=True)
            try:
                page = browser.new_page()
                page.goto(url + '/#describe')
                page.locator('#ai-model').select_option(label='Claude (subscription) — claude-test')
                with page.expect_request(lambda request: request.url.endswith('/ai')) as request_info:
                    page.locator('[data-action=ai]').click()
                request = request_info.value
                assert set(request.post_data_json) == {'uuid', 'source', 'model'}
                page.wait_for_function("document.querySelector('#tab-publish').textContent.includes('(1)')")
                row, = json.loads(workflow.jp.read_text())
                assert row['title'] == 'Generated title'
                assert row['description'] == 'Generated description'
                assert row['state'] == 'DESCRIBED'
            finally:
                browser.close()


def test_ai_failure_keeps_describe_card_and_retry_available(workflow, monkeypatch):
    from publicator.llm_meta import TODO_GENERATE
    from playwright.sync_api import sync_playwright

    def fail(*args, **kwargs):
        raise RuntimeError('stub provider failure')
    monkeypatch.setattr(workflow.gallery, 'generate_metadata', fail)
    with workflow.start() as url:
        uuid = pick(url, workflow)
        assert _post(url, {'uuid': uuid}, '/schedule/approve')[0] == 200
        with sync_playwright() as pw:
            browser = pw.firefox.launch(headless=True)
            try:
                page = browser.new_page()
                page.goto(url + '/#describe')
                page.locator('#ai-model').select_option(label='Claude (subscription) — claude-test')
                page.locator('[data-action=ai]').click()
                page.wait_for_function("!document.querySelector('[data-action=ai]').disabled")
                assert page.locator('[data-action=ai]').count() == 1
                assert page.locator('#describe-tab').inner_text().count('AI metadata pending') == 1
                row, = json.loads(workflow.jp.read_text())
                assert row['title'] == row['description'] == TODO_GENERATE
                assert row['state'] == 'SCHEDULED'
            finally:
                browser.close()


def test_custom_manual_schedule_without_profile_stays_custom_until_preset_choice(workflow):
    from playwright.sync_api import sync_playwright
    from publicator.config import validate_publications
    with workflow.start() as url:
        pick(url, workflow)
        rows = json.loads(workflow.jp.read_text())
        rows[0].pop('scheduleProfile')
        rows[0]['scheduleIsManual'] = True
        rows[0]['apparitionTimestampIfDifferentThanSubmission'] = 1800000000
        validate_publications(rows, workflow.cfg)
        workflow.jp.write_text(json.dumps(rows))
        before = workflow.jp.read_text()
        with sync_playwright() as pw:
            browser = pw.firefox.launch(headless=True)
            try:
                page = browser.new_page()
                page.goto(url + '/#schedule')
                preset = page.locator('.f-preset')
                assert preset.locator('option:checked').inner_text() == 'Custom / unassigned'
                assert preset.input_value() == ''
                assert page.locator('.f-schedule').input_value() == '2027-01-15T09:00'
                assert 'Manual pin' in page.locator('#schedule-tab').inner_text()
                page.reload()
                assert workflow.jp.read_text() == before
                with page.expect_response(lambda r: r.url.endswith('/schedule')):
                    preset.select_option(label='default')
                row, = json.loads(workflow.jp.read_text())
                assert row['scheduleProfile'] == 'default' and row['scheduleIsManual'] is False
                assert row['apparitionTimestampIfDifferentThanSubmission'] != 1800000000
            finally:
                browser.close()


@pytest.mark.parametrize('newer_field', ['title', 'description'])
def test_earlier_autosave_completion_preserves_newer_dirty_input(workflow, newer_field):
    """An in-flight partial save cannot clear a newer same/other-field edit."""
    from datetime import datetime, timedelta
    from playwright.sync_api import sync_playwright
    with workflow.start() as url:
        uuid = pick(url, workflow)
        assert _post(url, {'uuid': uuid}, '/schedule/approve')[0] == 200
        assert _post(url, {'uuid': uuid, 'title': 'Stored title', 'description': 'Stored description'}, '/metadata')[0] == 200
        with sync_playwright() as pw:
            browser = pw.firefox.launch(headless=True)
            try:
                page = browser.new_page()
                captured, errors = [], []
                page.on('pageerror', lambda error: errors.append(str(error)))
                def capture(route):
                    captured.append(route)
                    if len(captured) == 1:
                        page.evaluate('window.autosaveCaptured = true')
                    else:
                        route.continue_()
                page.route('**/metadata', capture)
                now = datetime(2027, 1, 1)
                page.clock.install(time=now)
                page.clock.pause_at(now + timedelta(seconds=1))
                page.goto(url + '/#describe')
                page.locator('[data-field=title]').fill('First edit')
                page.clock.run_for(350)
                page.wait_for_function('window.autosaveCaptured === true')
                assert captured[0].request.post_data_json == {'uuid': uuid, 'title': 'First edit'}
                page.locator(f'[data-field={newer_field}]').fill('Newer edit')
                with page.expect_response(lambda r: r.url.endswith('/metadata')):
                    captured[0].continue_()
                page.wait_for_function("document.querySelector('#describe-tab .status').textContent === 'Unsaved'")
                page.evaluate('() => saves')  # await the earlier completion, not just the input status
                assert page.evaluate('dirty.has(' + json.dumps(uuid) + ')')
                assert page.evaluate("values(document.querySelector('#describe-tab [data-uuid]'))") == {newer_field: 'Newer edit'}
                assert page.locator('#describe-tab .status').inner_text() == 'Unsaved'
                row, = json.loads(workflow.jp.read_text())
                assert row['title'] == 'First edit' and row['description'] == 'Stored description'
                with page.expect_response(lambda r: r.url.endswith('/metadata')):
                    page.clock.run_for(350)
                page.wait_for_function('dirty.size === 0')
                assert len(captured) == 2
                assert captured[1].request.post_data_json == {'uuid': uuid, newer_field: 'Newer edit'}
                row, = json.loads(workflow.jp.read_text())
                assert row[newer_field] == 'Newer edit'
                assert row['description' if newer_field == 'title' else 'title'] == ('Stored description' if newer_field == 'title' else 'First edit')
                assert page.locator('#describe-tab .status').inner_text() == 'Saved'
                assert not errors, errors
            finally:
                browser.close()


@pytest.mark.parametrize('profile', [None, 'removed'])
def test_migrated_custom_date_browser_return_repack_edit_and_suggest(workflow, profile):
    """Real migration, HTTP and Firefox; only providers/publishers are stubbed."""
    from playwright.sync_api import sync_playwright
    from publicator.migration import migrate_publications
    from publicator.scheduling import schedule_data
    workflow.cfg['schedule']['default_tiers'] = ['gold']
    uuid = '00000000-0000-4000-8000-000000000001'
    legacy = {'uuid': uuid, 'submissionTimestamp': 1700000000,
              'description': 'Saved description', 'files': [], 'apparitions': [{
                  'platformName': 'pixiv', 'state': 'unpublished', 'priceIfNotFree': 3,
                  'urlElsePublicationName': 'Saved title',
                  'apparitionTimestampIfDifferentThanSubmission': 1800000000}]}
    if profile is not None:
        legacy['scheduleProfile'] = profile
    rows = migrate_publications([legacy], {}, workflow.cfg)
    historical = rows[0]['apparitions'][0].copy()
    workflow.jp.write_text(json.dumps(rows))
    with workflow.start() as url, sync_playwright() as pw:
        browser = pw.firefox.launch(headless=True)
        try:
            page = browser.new_page()
            errors = []
            page.on('pageerror', lambda error: errors.append(str(error)))
            page.goto(url + '/#publish')
            with page.expect_navigation(wait_until='load'), page.expect_response(lambda r: r.url.endswith('/schedule/return')):
                page.locator(f'[data-uuid="{uuid}"] [data-action=return-schedule]').click()
            page.locator('#tab-pick').click()
            with page.expect_navigation(wait_until='load'), page.expect_response(lambda r: r.url.endswith('/pick')):
                page.locator('[data-action=pick]').first.click()
            page.locator('#tab-schedule').click()
            card = page.locator(f'[data-uuid="{uuid}"]')
            pub = json.loads(workflow.jp.read_text())[0]
            assert pub['apparitionTimestampIfDifferentThanSubmission'] == 1800000000
            assert pub.get('scheduleProfile') == profile and pub['scheduleIsManual'] is True
            assert card.locator('.f-preset option:checked').inner_text() == 'Custom / unassigned'
            assert card.locator('.f-schedule').input_value() == '2027-01-15T09:00'
            assert 'Manual pin' in card.inner_text()
            before = workflow.jp.read_bytes()
            page.reload()
            assert workflow.jp.read_bytes() == before
            with page.expect_navigation(wait_until='load'), page.expect_response(lambda r: r.url.endswith('/schedule')):
                card.locator('.f-schedule').fill('2027-01-16T11:00')
                card.locator('.f-schedule').press('Tab')
            page.wait_for_function("document.querySelector('.f-schedule').value === '2027-01-16T11:00'")
            pub = json.loads(workflow.jp.read_text())[0]
            assert pub.get('scheduleProfile') == profile and pub['scheduleIsManual'] is True
            assert pub['title'] == 'Saved title' and pub['description'] == 'Saved description'
            assert pub['apparitions'] == [historical]
            assert card.locator('.f-preset option:checked').inner_text() == 'Custom / unassigned'
            with page.expect_navigation(wait_until='load'), page.expect_response(lambda r: r.url.endswith('/schedule/approve')):
                card.locator('[data-action=approve-date]').click()
            page.locator('#tab-describe').click()
            with page.expect_navigation(wait_until='load'), page.expect_response(lambda r: r.url.endswith('/metadata/approve')):
                card.locator('[data-action=approve-metadata]').click()
            page.locator('#tab-publish').click()
            with page.expect_navigation(wait_until='load'), page.expect_response(lambda r: r.url.endswith('/selection')):
                card.locator('[data-platform=deviantart]').check()
            pub = json.loads(workflow.jp.read_text())[0]
            assert pub['apparitions'] == [historical, {'platformName': 'deviantart', 'state': 'unpublished'}]
            with page.expect_navigation(wait_until='load'), page.expect_response(lambda r: r.url.endswith('/schedule/return')):
                card.locator('[data-action=return-schedule]').click()
            page.locator('#tab-schedule').click()
            with page.expect_navigation(wait_until='load'), page.expect_response(lambda r: r.url.endswith('/schedule')):
                card.locator('[data-action=suggest]').click()
            page.wait_for_function("document.querySelector('.f-preset').value === 'default'")
            pub = json.loads(workflow.jp.read_text())[0]
            assert pub['scheduleProfile'] == 'default' and pub['scheduleIsManual'] is False
            assert pub['apparitionTimestampIfDifferentThanSubmission'] in schedule_data(workflow.cfg['schedule'])[0]['slots']
            assert pub['apparitions'][0] == historical
            assert not workflow.submitted and not errors, errors
        finally:
            browser.close()
