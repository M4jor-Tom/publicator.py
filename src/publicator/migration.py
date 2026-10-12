"""Pure publications.json migration to the four-phase publication contract."""

from __future__ import annotations

import copy

from publicator.config import is_http_url, validate_publications
from publicator.entries import (
    APPARITION_STATE_PUBLISHED,
    PUBLICATION_STATE_APPEARED,
    PUBLICATION_STATE_DESCRIBED,
    PUBLICATION_STATE_PICKED,
    PUBLICATION_STATE_SCHEDULED,
    PUBLICATION_STATES,
)
from publicator.llm_meta import TODO_GENERATE

_OBSOLETE_TITLE = "urlElsePublicationName"
_OBSOLETE_DATE = "apparitionTimestampIfDifferentThanSubmission"


def _complete_metadata(pub: dict) -> bool:
    return all(pub.get(k, "").strip() and TODO_GENERATE not in pub[k]
               for k in ("title", "description"))


def _infer_state(pub: dict) -> str:
    if pub.get("state") in PUBLICATION_STATES:
        return pub["state"]
    apps = pub.get("apparitions", [])
    if apps and all(a.get("state") == APPARITION_STATE_PUBLISHED for a in apps):
        return PUBLICATION_STATE_APPEARED
    if pub.get(_OBSOLETE_DATE) is not None and _complete_metadata(pub):
        return PUBLICATION_STATE_DESCRIBED
    if pub.get(_OBSOLETE_DATE) is not None:
        return PUBLICATION_STATE_SCHEDULED
    return PUBLICATION_STATE_PICKED


def migrate_publications(publications: list[dict], recovered_titles: dict[str, str],
                         config: dict) -> list[dict]:
    """Return a migrated deep copy; never mutates `publications`."""
    out = copy.deepcopy(publications)
    for pub in out:
        new_draft = (
            pub.get("state") in {PUBLICATION_STATE_PICKED, PUBLICATION_STATE_SCHEDULED}
            and "title" in pub
            and not any(_OBSOLETE_TITLE in app or _OBSOLETE_DATE in app
                        for app in pub.get("apparitions", []))
        )
        legacy_names: set[str] = set()
        legacy_dates: set[int] = set()
        for app in pub.get("apparitions", []):
            old_name = app.pop(_OBSOLETE_TITLE, None)
            if old_name and old_name.strip():
                if is_http_url(old_name):
                    if app.get("url") not in (None, old_name):
                        raise ValueError(f"{pub.get('uuid')}: conflicting apparition urls")
                    app["url"] = old_name
                else:
                    legacy_names.add(old_name)
            if _OBSOLETE_DATE in app:
                legacy_dates.add(int(app.pop(_OBSOLETE_DATE)))

        if len(legacy_names) > 1:
            raise ValueError(f"{pub.get('uuid')}: conflicting legacy titles")
        if len(legacy_dates) > 1:
            raise ValueError(f"{pub.get('uuid')}: conflicting apparition dates")
        if legacy_dates:
            ts = next(iter(legacy_dates))
            existing = pub.get(_OBSOLETE_DATE)
            if existing is not None and int(existing) != ts:
                raise ValueError(f"{pub.get('uuid')}: conflicting apparition dates")
            pub[_OBSOLETE_DATE] = ts

        current_title = pub.get("title", "")
        if current_title.strip() and legacy_names and current_title not in legacy_names:
            raise ValueError(f"{pub.get('uuid')}: conflicting legacy titles")
        if not current_title.strip():
            if legacy_names:
                pub["title"] = next(iter(legacy_names))
            elif not new_draft:
                recovered = recovered_titles.get(pub.get("uuid"))
                if not recovered or not recovered.strip():
                    raise ValueError(f"{pub.get('uuid')}: recovered title required")
                pub["title"] = recovered

        pub.setdefault("apparitions", [])
        pub.setdefault("description", "")
        pub["state"] = _infer_state(pub)

    validate_publications(out, config)
    return out
