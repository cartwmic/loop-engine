pub fn open_store() {
    let _connection = rusqlite::Connection::open("store.db");
}
