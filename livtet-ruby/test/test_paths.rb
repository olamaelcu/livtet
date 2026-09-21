# frozen_string_literal: true

require "minitest/autorun"
require "livtet"

class TestPaths < Minitest::Test
  def test_returns_symbol_keys
    assert_equal %i[bundle config data logs].sort, Livtet.paths.keys.sort
  end

  def test_bundle_id
    assert_equal "net.olamaelcu.livtet", Livtet.paths[:bundle]
  end

  def test_values_are_strings
    Livtet.paths.each_value { |v| assert_instance_of String, v }
  end
end
