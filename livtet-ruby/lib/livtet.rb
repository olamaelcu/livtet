# frozen_string_literal: true

require_relative "livtet/version"
require_relative "livtet/livtet"

module Livtet
  class << self
    alias_method :native_paths, :paths

    # => { bundle:, data:, config:, logs: } (symbol keys)
    def paths
      native_paths.transform_keys(&:to_sym)
    end
  end

  # Handle to the livtet database. v1 exposes open/close only —
  # no query APIs. One database may be open at a time per process.
  class Database
    def self.open(path = nil)
      unless path.nil? || path.is_a?(String)
        raise TypeError, "path must be a String or nil"
      end

      native_open(path)
      db = allocate
      return db unless block_given?

      begin
        yield db
      ensure
        db.close unless db.closed?
      end
    end

    def path
      native_path
    end

    def close
      native_close
    end

    def closed?
      native_closed
    end
  end
end
