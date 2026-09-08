"""Environment owned by a fixture, with no inherited account configuration."""
import os


def fixture_environment(work, **overrides):
    environment = {key: os.environ[key] for key in
                   ('PATH', 'LANG', 'LC_ALL', 'LC_CTYPE', 'TZ', 'TERM',
                    'MISA_TREE_SITTER_DIR') if key in os.environ}
    environment.update(HOME=str(work), MISA_FIXTURE_ROOT=str(work), CODEX_HOME=str(work / 'codex'),
                       CLAUDE_CONFIG_DIR=str(work / 'claude-config'),
                       XDG_CONFIG_HOME=str(work / 'config'),
                       XDG_STATE_HOME=str(work / 'state-home'),
                       XDG_CACHE_HOME=str(work / 'cache'),
                       XDG_DATA_HOME=str(work / 'data'),
                       MISA_AUTH_FILE=str(work / 'auth'),
                       MISA_STATE_FILE=str(work / 'state'))
    environment.update(overrides)
    return environment
