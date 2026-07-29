local provider = {
  id = "legacy-provider",
  name = "Legacy Provider",
  version = "0.1.0",
  capabilities = { link_resolver = true },
}

function provider.resolve_links(urn, options)
  return { links = {} }
end

return provider
