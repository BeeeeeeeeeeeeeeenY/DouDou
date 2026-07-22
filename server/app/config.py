import os

SERVER_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def resolve_data_dir(data_dir: str | None) -> str:
    d = data_dir or os.environ.get("DOUDOU_DATA") or os.path.join(SERVER_DIR, "data")
    for sub in ("", "images", "audio"):
        os.makedirs(os.path.join(d, sub), exist_ok=True)
    return d
