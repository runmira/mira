# Homebrew formula for Mira.
#
# This file is the source of truth — copy it into the runmira/homebrew-tap
# repository (Formula/mira.rb) so users can:
#
#   brew install runmira/tap/mira
#
# The release workflow updates url/sha256 for each new tag.
class Mira < Formula
  desc "Open-source coding agent you run yourself, with the model you choose"
  homepage "https://github.com/runmira/mira"
  version "0.3.9"
  license "Apache-2.0"

  on_macos do
    on_arm do
      url "https://github.com/runmira/mira/releases/download/v#{version}/mira-darwin-arm64.tar.gz"
      sha256 "20386540bdba0c40766ed50cbfca1b85d9996d8126ea7a91ac3c76827dcf7bdc"
    end

    on_intel do
      url "https://github.com/runmira/mira/releases/download/v#{version}/mira-darwin-x86_64.tar.gz"
      sha256 "5be78a8ccd874793e3c42ce6b1aaa0cbad8feede56a80bc7704e12e53607cb3e"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/runmira/mira/releases/download/v#{version}/mira-linux-arm64.tar.gz"
      sha256 "446e2346fd7c6ccc04f2eb2b250bff8596b9c9fdcc234e60ee793ca2e0b287de"
    end

    on_intel do
      url "https://github.com/runmira/mira/releases/download/v#{version}/mira-linux-x86_64.tar.gz"
      sha256 "4ce8174c3e4d01a0857dc13d14b4ed4cab9a66cd323309c1d53216bf1078ea5c"
    end
  end

  def install
    bin.install "mira"
  end

  test do
    assert_match "mira", shell_output("#{bin}/mira --version")
  end
end
