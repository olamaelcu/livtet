# frozen_string_literal: true

require "minitest/autorun"
require "tmpdir"
require "livtet"

class TestDatabase < Minitest::Test
  def teardown
    # Never leak the process-global handle between tests.
    Livtet::Database.allocate.close
  rescue Livtet::Error, StandardError
    nil
  end

  def with_tmpdb
    Dir.mktmpdir do |dir|
      yield File.join(dir, "test.db")
    end
  end

  def test_open_close_explicit
    with_tmpdb do |path|
      db = Livtet::Database.open(path)
      assert_equal path, db.path
      refute db.closed?
      db.close
      assert db.closed?
      assert File.exist?(path)
    end
  end

  def test_block_form_auto_closes
    with_tmpdb do |path|
      result = Livtet::Database.open(path) do |db|
        assert_equal path, db.path
        :ok
      end
      assert_equal :ok, result
    end
  end

  def test_block_form_closes_on_error
    with_tmpdb do |path|
      assert_raises(RuntimeError) do
        Livtet::Database.open(path) { |_| raise "boom" }
      end
      # Handle released: a fresh open must succeed.
      Livtet::Database.open(path).close
    end
  end

  def test_close_twice_raises
    with_tmpdb do |path|
      db = Livtet::Database.open(path)
      db.close
      assert_raises(Livtet::Error) { db.close }
    end
  end

  def test_path_after_close_raises
    with_tmpdb do |path|
      db = Livtet::Database.open(path)
      db.close
      assert_raises(Livtet::Error) { db.path }
    end
  end

  def test_rejects_non_string_path
    assert_raises(TypeError) { Livtet::Database.open(123) }
  end

  def test_error_is_standard_error
    assert Livtet::Error < StandardError
  end
end
