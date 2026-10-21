"""Enable defused XML parsing for the stdlib XML-RPC compatibility client."""

from __future__ import annotations

from defusedxml.xmlrpc import monkey_patch

# Patch before any project code reads an untrusted rTorrent XML-RPC response.
monkey_patch()

# nosemgrep: python.lang.security.use-defused-xmlrpc.use-defused-xmlrpc -- The stdlib client is patched above; its outbound Binary and error APIs are still needed.
import xmlrpc.client as xmlrpc_client
