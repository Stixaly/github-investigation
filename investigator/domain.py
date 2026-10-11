import re
from urllib.parse import urlsplit


def parse_seed(value: str) -> tuple[str, str]:
    value = value.strip().rstrip("/")
    if "://" in value:
        url = urlsplit(value)
        if url.scheme != "https" or url.netloc.lower() != "github.com" or url.query or url.fragment:
            raise ValueError("Utiliser une URL https://github.com/compte[/depot] sans paramètres")
        value = url.path.strip("/")
    parts = value.split("/")
    if len(parts) not in (1, 2) or not re.fullmatch(r"[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})", parts[0]):
        raise ValueError("Seed attendue : compte ou compte/depot")
    if len(parts) == 2 and (not re.fullmatch(r"[A-Za-z0-9_.-]{1,100}", parts[1]) or parts[1] in (".", "..")):
        raise ValueError("Nom de dépôt incorrect")
    return ("Repository" if len(parts) == 2 else "Account", "/".join(parts))
