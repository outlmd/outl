//! Rename and merge a page. Its own macro because `catalog.rs` sits at its
//! size ceiling, the same reason `wrappers/trash.rs` exists. Both clients
//! invoke it from their `commands/page.rs`, next to `page_commands!`.

/// `rename_page` / `merge_pages`: change a page's name, or fold one page
/// into another (RFC 0557).
#[macro_export]
macro_rules! page_merge_commands {
    ($state:ty) => {
        $crate::tauri_commands! {
            state = $state;

            /// Rename a page (slug or id) to human text and rewrite every
            /// link to it. A name another page owns is a typed reply, not
            /// an error, and writes nothing.
            async fn rename_page(page: String, new_name: String)
                -> ::std::result::Result<$crate::commands::page_merge::RenamePageReply, String>
                => $crate::commands::page_merge::rename_page;

            /// Fold one page into another and rewrite links to the source.
            async fn merge_pages(source: String, target: String)
                -> ::std::result::Result<$crate::commands::page_merge::MergePagesReply, String>
                => $crate::commands::page_merge::merge_pages;
        }
    };
}
