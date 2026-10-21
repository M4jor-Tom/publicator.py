from publicator_da.browser import validate_profiles


def test_profile_trees_must_be_disjoint(tmp_path):
    source = tmp_path / "login"
    work = tmp_path / "work"
    source.mkdir()
    work.mkdir()
    validate_profiles(source, work)
    for a, b in ((source, source), (source, source / "nested"), (source / "nested", source)):
        try:
            validate_profiles(a, b)
        except ValueError:
            pass
        else:
            raise AssertionError(f"overlapping profiles accepted: {a}, {b}")
