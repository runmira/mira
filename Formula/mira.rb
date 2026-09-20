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
  version "0.3.6"
  license "Apache-2.0"

  on_macos do
    on_arm do
      url "https://github.com/runmira/mira/releases/download/v#{version}/mira-darwin-arm64.tar.gz"
      sha256 "8393056129d4cb8d2b263de7373ba3169f406c1c136a034a0717765f97c8af61"
    end

    on_intel do
      url "https://github.com/runmira/mira/releases/download/v#{version}/mira-darwin-x86_64.tar.gz"
      sha256 "afa29ced8716beef54bd260f0fea17f7839fbb70f95f27d05da297ce096b94a2"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/runmira/mira/releases/download/v#{version}/mira-linux-arm64.tar.gz"
      sha256 "8420d4f845bee61e5a35bb856d5f630b25f84d17eb8855edb87965bcbca718e4"
    end

    on_intel do
      url "https://github.com/runmira/mira/releases/download/v#{version}/mira-linux-x86_64.tar.gz"
      sha256 "5f7100da151ec24d91813866fa3388614e1c90bf5940738f1f4f08fc4cdbba2c"
    end
  end

  def install
    bin.install "mira"
  end

  test do
    assert_match "mira", shell_output("#{bin}/mira --version")
  end
end
