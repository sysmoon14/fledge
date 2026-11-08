"""The Fledge integration's tests, against a real Home Assistant core.

    uv venv --python 3.13 .venv && uv pip install pytest-homeassistant-custom-component
    .venv/bin/pytest integrations/home-assistant/tests      # from the repository's root
"""

import pytest

pytest_plugins = "pytest_homeassistant_custom_component"


@pytest.fixture(autouse=True)
def auto_enable_custom_integrations(enable_custom_integrations):
    yield
