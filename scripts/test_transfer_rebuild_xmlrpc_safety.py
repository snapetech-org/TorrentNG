#!/usr/bin/env python3
"""Regression tests for the defused XML-RPC compatibility wrapper."""
from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from defusedxml.common import EntitiesForbidden
# nosemgrep: python.lang.security.use-defused-xmlrpc.use-defused-xmlrpc -- this regression test exercises the shared wrapper that monkey-patches the stdlib parser with defusedxml.
from transfer_rebuild.xmlrpc_safety import xmlrpc_client


class XmlRpcSafetyTests(unittest.TestCase):
    def test_xmlrpc_responses_reject_entity_expansion(self):
        payload = b"""<?xml version=\"1.0\"?>
        <!DOCTYPE methodResponse [<!ENTITY expansion \"blocked\">]>
        <methodResponse><params><param><value><string>&expansion;</string></value></param></params></methodResponse>
        """

        with self.assertRaises(EntitiesForbidden):
            xmlrpc_client.loads(payload)


if __name__ == "__main__":
    unittest.main()
