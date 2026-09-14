class Seal < Formula
  desc "Cross-platform secrets manager (CLI + GUI) backed by the OS keychain"
  homepage "https://github.com/bucabay/seal"
  url "https://github.com/bucabay/seal/archive/refs/tags/v0.2.0.tar.gz"
  sha256 "30767495a3878f9c06ecd7c609634b8e9e20eba83356cf4804abaf093dde36b7"
  license "MIT"
  head "https://github.com/bucabay/seal.git", branch: "main"

  depends_on "rust" => :build

  on_linux do
    depends_on "dbus"
    depends_on "libsecret"
  end

  def install
    # CLI-only build: no Tauri/GUI/frontend, no C compilation.
    # The GUI is distributed separately (see --cask, once published).
    system "cargo", "build",
           "--manifest-path", "src-tauri/Cargo.toml",
           "--no-default-features",
           "--release"

    bin.install "src-tauri/target/release/seal"

    # Install the agent skill
    (share/"seal/skills/seal").install "skills/seal/SKILL.md"
  end

  def caveats
    <<~EOS
      To enable the Seal agent skill (Claude Code / opencode):
        ln -s "#{opt_share}/seal/skills/seal" "$HOME/.claude/skills/seal"
    EOS
  end

  test do
    system "#{bin}/seal", "--help"
    # Guards against the formula and the installed binary drifting apart.
    assert_match version.to_s, shell_output("#{bin}/seal --version")
    assert_path_exists share/"seal/skills/seal/SKILL.md"
  end
end
