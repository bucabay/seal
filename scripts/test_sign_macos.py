"""Signing preflight tests; no real identity, profile, or secret is used."""
import datetime
import hashlib
import importlib.util
import pathlib
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("sign_macos", pathlib.Path(__file__).with_name("sign-macos.py"))
sign = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sign)


class SigningTests(unittest.TestCase):
    def setUp(self):
        self.team = "ABCDEFGHIJ"
        self.certificate = b"synthetic-test-certificate"
        self.cert_hash = hashlib.sha1(self.certificate).hexdigest()
        self.profile = {
            "Platform": ["OSX"],
            "Entitlements": {
                "com.apple.developer.team-identifier": self.team,
                "com.apple.application-identifier": self.team + ".app.seal.*",
                "keychain-access-groups": [self.team + ".*"],
            },
            "ExpirationDate": datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(days=1),
            "DeveloperCertificates": [self.certificate],
        }

    def validate(self):
        return sign.validate_profile(self.profile, self.team, "app.seal.secure", self.cert_hash)

    def test_matching_profile_authorizes_shared_group(self):
        self.assertEqual(self.validate()["keychain-access-groups"], [self.team + ".app.seal.secure.secrets"])

    def test_ios_profile_is_refused_even_with_matching_identity_and_groups(self):
        self.profile["Platform"] = ["iOS", "xrOS", "visionOS"]
        with self.assertRaisesRegex(ValueError, "iOS profile cannot authorize"):
            self.validate()

    def test_other_team_is_refused(self):
        self.profile["Entitlements"]["com.apple.developer.team-identifier"] = "OTHERTEAM1"
        with self.assertRaisesRegex(ValueError, "signing team"):
            self.validate()

    def test_other_app_id_is_refused(self):
        self.profile["Entitlements"]["com.apple.application-identifier"] = self.team + ".other.app"
        with self.assertRaisesRegex(ValueError, "bundle ID"):
            self.validate()

    def test_unauthorized_group_is_refused(self):
        self.profile["Entitlements"]["keychain-access-groups"] = [self.team + ".other.group"]
        with self.assertRaisesRegex(ValueError, "keychain group"):
            self.validate()

    def test_other_certificate_is_refused(self):
        self.profile["DeveloperCertificates"] = [b"other"]
        with self.assertRaisesRegex(ValueError, "signing certificate"):
            self.validate()

    def test_expired_profile_is_refused(self):
        self.profile["ExpirationDate"] = datetime.datetime.now() - datetime.timedelta(days=1)
        with self.assertRaisesRegex(ValueError, "expired"):
            self.validate()

    def test_cli_is_wrapped_without_changing_original(self):
        with tempfile.TemporaryDirectory(prefix="seal-sign-test-") as tmp:
            source = pathlib.Path(tmp) / "seal"
            source.write_bytes(b"test-executable")
            bundle, executable = sign.cli_bundle(source, "app.seal.secure")
            self.assertEqual(source.read_bytes(), b"test-executable")
            self.assertEqual(executable.read_bytes(), source.read_bytes())
            self.assertEqual(executable.parent.parent.parent, bundle)
            with self.assertRaisesRegex(ValueError, "unrelated bundle"):
                sign.cli_bundle(source, "other.app")


if __name__ == "__main__":
    unittest.main()
