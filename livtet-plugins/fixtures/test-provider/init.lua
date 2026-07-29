local provider = {
  id = "test-provider",
  name = "Test Provider",
  version = "1.0.0",
  capabilities = { link_resolver = true },
  requires = { http = true },
}

function provider.resolve_links(urn, options)
  return {
    links = {
      {
        label = "Test Link",
        url = "https://example.com/book?urn=" .. urn,
        category = "reference",
        sort_hint = 100,
      },
    },
  }
end

return provider
