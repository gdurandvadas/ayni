from main import greet


def test_greet():
    assert greet("Ada") == {"message": "Hi, Ada! Number=9"}
