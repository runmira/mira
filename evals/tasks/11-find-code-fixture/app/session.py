import secrets

SESSION_TTL_SECONDS = 60 * 60 * 12


def new_session_token():
    return secrets.token_urlsafe(32)
