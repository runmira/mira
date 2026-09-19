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
  version "0.3.1"
  license "Apache-2.0"

  on_macos do
    on_arm do
      url "https://github.com/runmira/mira/releases/download/v#{version}/mira-darwin-arm64.tar.gz"
      sha256 "a906f2bc3283a99ee599a9d009a0a32d8e9ca5257bdc64f2be7ad8d18b55ba89"
    end

    on_intel do
      url "https://github.com/runmira/mira/releases/download/v#{version}/mira-darwin-x86_64.tar.gz"
      sha256 "0b872ef1967e52278c346384fc61331b301501d63122ac48b64d17c2e23ec79c"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/runmira/mira/releases/download/v#{version}/mira-linux-arm64.tar.gz"
      sha256 "d769f48a6d77f34e1646cc4ea54d19da0453dd13115519d85b5267d49e625326"
    end

    on_intel do
      url "https://github.com/runmira/mira/releases/download/v#{version}/mira-linux-x86_64.tar.gz"
      sha256 "240dac8119540f4260f60daa853b0fbe35992bb74308ace676d0bbd1608f2474"
    end
  end

  def install
    bin.install "mira"
  end

  test do
    assert_match "mira", shell_output("#{bin}/mira --version")
  end
end
