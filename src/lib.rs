use scrypto::prelude::*;

#[derive(ScryptoSbor, NonFungibleData)]
struct OwnerBadge {
    pub name: String,
}

#[derive(ScryptoSbor, NonFungibleData)]
struct AdminBadge {
    pub name: String,
}

#[blueprint]
mod nft_sale {

    enable_method_auth! {
        roles {
            admin => updatable_by: [OWNER];
        },
        methods {
            start_sale => restrict_to: [admin, OWNER];
            end_sale => restrict_to: [admin, OWNER];
            change_price => restrict_to: [admin, OWNER];
            withdraw_profits => restrict_to: [OWNER];
            add_nfts_to_vault => restrict_to: [admin, OWNER];
            price => PUBLIC;
            is_sold => PUBLIC;
            buy => PUBLIC;
        }
    }

    struct NFTSale {
        // Vault to hold the NFT collection
        nft_vault: NonFungibleVault,
        // Vault to hold the xrd paid for NFTs
        xrd_vault: Vault,
        // The token to accept as payment
        accepted_payment_token: ResourceAddress,
        // Price per NFT in 'accepted_payment_token'
        price: Decimal,

        admin_badge_address: ResourceAddress,

        sale_allowed: bool,
    }

    impl NFTSale {
        pub fn instantiate_nft_sale(
            nft_resource_address: ResourceAddress,
            accepted_payment_token: ResourceAddress,
            price: Decimal,
        ) -> (Global<NFTSale>, NonFungibleBucket, NonFungibleBucket) {
            let (address_reservation, component_address) =
                Runtime::allocate_component_address(<NFTSale>::blueprint_id());

            assert!(
                !matches!(
                    ResourceManager::from_address(accepted_payment_token).resource_type(),
                    ResourceType::NonFungible { id_type: _ }
                ),
                "Only payments of fungible resources are accepted."
            );
            assert!(
                price >= Decimal::zero(),
                "The price cannot be less then ZERO!"
            );


            let owner_badge: NonFungibleBucket = ResourceBuilder::new_integer_non_fungible::<OwnerBadge>(OwnerRole::None)
                .metadata(metadata!{
                    init {
                        "name" => "Component Owner Badge", locked;
                        "icon_url" => Url::of("https://s2.coinmarketcap.com/static/img/coins/200x200/11948.png"/*Placeholder Image*/), updatable;
                        "tags" => "badge", locked;
                    }
                })
                .mint_roles(mint_roles!{
                    // This rule says only the component itself can mint these non fungibles
                    minter => rule!(require(global_caller(component_address)));
                    // no one can update the minter role
                    minter_updater => rule!(deny_all);
                })
                .mint_initial_supply([
                    (0u64.into(), OwnerBadge { name: "Owner Badge".to_owned()}),
                ]);

            // Create admin badges for team members to interact with a few of the auth protected methods
            let admin_badge: NonFungibleBucket = ResourceBuilder::new_integer_non_fungible::<AdminBadge>(OwnerRole::None)
                .metadata(metadata!{
                    init {
                        "name" => "Component Admin Badge", locked;
                        "icon_url" => Url::of("https://s2.coinmarketcap.com/static/img/coins/200x200/11948.png"/*Placeholder Image*/), updatable;
                        "tags" => "badge", locked;
                    }
                })
                .mint_roles(mint_roles!{
                    // This rule says only the component itself can mint these non fungibles
                    minter => rule!(require(global_caller(component_address)));
                    // no one can update the minter role
                    minter_updater => rule!(deny_all);
                })
                .recall_roles(recall_roles!{
                    // Owner can recall the admin badges
                    recaller => rule!(require(global_caller(owner_badge.resource_address())));
                    // no one can update the recaller role
                    recaller_updater => rule!(require(global_caller(owner_badge.resource_address())));
                })
                .mint_initial_supply([
                    (0u64.into(), AdminBadge { name: "Admin Badge".to_owned()}),
                ]);

            let component_address = Self {
                nft_vault: NonFungibleVault::new(nft_resource_address),
                xrd_vault: Vault::new(accepted_payment_token),
                accepted_payment_token,
                price,
                admin_badge_address: admin_badge.resource_address(),
                sale_allowed: false,
            }
            .instantiate()
            .prepare_to_globalize(OwnerRole::Fixed(rule!(require(
                owner_badge.resource_address()
            ))))
            .with_address(address_reservation)
            .roles(roles!(
                admin => rule!(require(admin_badge.resource_address()));
            ))
            .globalize();

            (component_address, owner_badge, admin_badge)
        }

        // Add nfts to the nft vault after instatiation for testing methods
        pub fn add_nfts_to_vault(&mut self, nft_deposit_bucket: NonFungibleBucket) {
            // Add the bucket to the vault
            self.nft_vault.put(nft_deposit_bucket)
        }


        // Set the bool to true so the sale can begin
        pub fn start_sale(&mut self) {
            self.sale_allowed = true;
        }

        // Set the bool back to false so the sale will no longer be available
        pub fn end_sale(&mut self) {
            self.sale_allowed = false;
        }

        // Buy the specified number of NFTs and supply payment
        pub fn buy(
            &mut self,
            mut payment: Bucket,
            number_of_nfts: u16,
        ) -> (Bucket, NonFungibleBucket) {
            // Check if the sale is allowed
            assert!(
                self.sale_allowed,
                "[Buy]: Sale is not allowed yet. Please wait until the sale starts."
            );
            // Verify the token supplied is the correct resource
            assert_eq!(
                payment.resource_address(),
                self.accepted_payment_token,
                "[Buy]: Invalid token provided. Payment is only accepted in {:?}",
                self.accepted_payment_token
            );
            // Enforce the limit of 10 NFTs per purchase
            assert!(
                number_of_nfts <= 10,
                "[Buy]: You can only buy a maximum of 10 NFTs per transaction."
            );

            // Verify the amount supplied is correct
            assert!(
                payment.amount() >= self.price * number_of_nfts,
                "[Buy]: Invalid quantity was provided. This sale can only go through when {} tokens are provided.",
                self.price * number_of_nfts
            );

            // Take the given number of NFTs specified by the user from the vault
            let nft = self.nft_vault.take(number_of_nfts);

            // Take the required amount of tokens for the purchase (without change)
            let revenue = payment.take(self.price * number_of_nfts);

            // Store it in the xrd vault
            self.xrd_vault.put(revenue);

            // Return any excess funds and the bucket of NFTs purchased
            (payment, nft)
        }


        // Once the vault holds some funds they can be withdrawn using this method
        pub fn withdraw_profits(&mut self) -> Bucket {
            // Check if the tokens have been sold or not
            assert!(
                self.is_sold(),
                "[Withdraw Payment]: Cannot withdraw funds when the payment vault is empty."
            );
            return self.xrd_vault.take_all();
        }

        // Re set the price from the original set at instantiation
        pub fn change_price(&mut self, price: Decimal) {
            // Checking that the new price can be set
            assert!(
                price >= Decimal::zero(),
                "[Change Price]: The tokens can not be sold for a negative amount."
            );
            self.price = price;
        }

        // Returns the current price of the nft
        pub fn price(&self) -> (ResourceAddress, Decimal) {
            return (self.accepted_payment_token, self.price);
        }

        // Check the xrd vault to verify if any sales have happened yet
        pub fn is_sold(&self) -> bool {
            return !self.xrd_vault.is_empty();
        }
    }
}
