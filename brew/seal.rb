class Seal < Formula
  desc "Cross-platform secrets manager (CLI + GUI) backed by the OS keychain"
  homepage "https://github.com/bucabay/seal"
  url "https://github.com/bucabay/seal/archive/refs/tags/v0.1.2.tar.gz"
  sha256 "2557872f1eb46b9a85b4a75d25df659b9e2b3f19bdbc90a7d50f4d55adc69b1a"
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
    assert_predicate share/"seal/skills/seal/SKILL.md", :exist?
  end
end
