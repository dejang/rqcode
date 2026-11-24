let vault = Vault.create(secretKey: "prod-secret")
_ = vault.verify("prod-secret")
