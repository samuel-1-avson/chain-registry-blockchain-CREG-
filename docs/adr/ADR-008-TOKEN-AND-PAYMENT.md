# ADR-008: Token and payment decision

- **Context/problem:** current rewards are below real validation cost and publisher liability is weak.
- **Options:** native token first; unpaid volunteers; priced fiat/stablecoin/credits with bonds.
- **Decision:** MVP uses measured job pricing and non-speculative bonds; no tradable token in the security path.
- **Security implications:** economics do not depend on market price or mint authority.
- **Operational implications:** meter CPU, memory, storage, egress, review, and incident work.
- **Migration implications:** legacy token contracts are read-only/treasury-managed until reviewed.
- **Rejected alternatives:** price assumptions disguised as validator incentives.
- **Revisit conditions:** real demand, independent operators, enforceable slashing, and legal review.
