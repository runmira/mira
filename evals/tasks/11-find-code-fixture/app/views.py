from app.auth import check_password
from app.session import new_session_token


def login(user, password):
    if not check_password(password, user.salt, user.password_hash):
        return None
    return new_session_token()
