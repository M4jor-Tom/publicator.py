import pytest

from publicator import deviantart, entries


@pytest.fixture(autouse=True)
def _restore_data_dir_globals():
    """configure() mutates the process-global DATA_DIR (and friends) on both
    deviantart and entries; restore them so a tmp_path pointed at by one test
    doesn't leak (deleted) into the next."""
    saved = (deviantart.DATA_DIR, deviantart.LOGIN_DIR, deviantart.SESSION_DIR,
             deviantart.TAGS_FILE, entries.DATA_DIR)
    try:
        yield
    finally:
        (deviantart.DATA_DIR, deviantart.LOGIN_DIR, deviantart.SESSION_DIR,
         deviantart.TAGS_FILE, entries.DATA_DIR) = saved
